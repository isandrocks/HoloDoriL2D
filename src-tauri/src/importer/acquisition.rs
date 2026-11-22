use futures_util::StreamExt;
use md5::{Digest, Md5};
use std::fs::{self, File};
use std::io::Write;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use tracing::{debug, info};

use super::cache::ImporterCacheManager;
use crate::domain::error::{DomainError, ErrorCode};

pub const DEFAULT_CDN_URL_TEMPLATE: &str =
    "https://asset.review-game-hololive-dreams.com/{objectName}";

pub struct AssetAcquisition {
    cdn_template: String,
    client: reqwest::Client,
}

impl Default for AssetAcquisition {
    fn default() -> Self {
        Self::new()
    }
}

impl AssetAcquisition {
    pub fn new() -> Self {
        Self {
            cdn_template: DEFAULT_CDN_URL_TEMPLATE.to_string(),
            client: reqwest::Client::builder()
                .timeout(std::time::Duration::from_secs(60))
                .pool_max_idle_per_host(0)
                .build()
                .unwrap_or_default(),
        }
    }

    pub fn with_cdn_template(template: impl Into<String>) -> Self {
        Self {
            cdn_template: template.into(),
            client: reqwest::Client::builder()
                .timeout(std::time::Duration::from_secs(60))
                .pool_max_idle_per_host(0)
                .build()
                .unwrap_or_default(),
        }
    }

    pub fn with_client(mut self, client: reqwest::Client) -> Self {
        self.client = client;
        self
    }

    /// Acquires a bundle, checking local cache first. If not cached, downloads from CDN.
    pub async fn acquire_bundle<F>(
        &self,
        object_name: &str,
        expected_md5: &str,
        expected_size: Option<u64>,
        cache_manager: &ImporterCacheManager,
        cancel_token: Arc<AtomicBool>,
        on_progress: F,
    ) -> Result<PathBuf, DomainError>
    where
        F: FnMut(u64, u64),
    {
        self.acquire_bundle_for_asset(
            object_name,
            None,
            expected_md5,
            expected_size,
            cache_manager,
            cancel_token,
            on_progress,
        )
        .await
    }

    /// Acquires a bundle for a catalog asset, searching `cache/bundles/` first for `object_name`
    /// or an exact extensionless match to `asset_name` (used directly as if it had `.bundle`),
    /// and falling back to CDN download if not found in cache.
    pub async fn acquire_bundle_for_asset<F>(
        &self,
        object_name: &str,
        asset_name: Option<&str>,
        expected_md5: &str,
        expected_size: Option<u64>,
        cache_manager: &ImporterCacheManager,
        cancel_token: Arc<AtomicBool>,
        mut on_progress: F,
    ) -> Result<PathBuf, DomainError>
    where
        F: FnMut(u64, u64),
    {
        cache_manager.ensure_dirs()?;

        if let Some(cached_path) =
            self.find_cached_bundle(object_name, asset_name, expected_md5, cache_manager)
        {
            debug!(
                "Bundle {} (asset: {:?}) hit local cache: {}",
                object_name,
                asset_name,
                cached_path.display()
            );
            if let Ok(meta) = fs::metadata(&cached_path) {
                on_progress(meta.len(), meta.len());
            }
            return Ok(cached_path);
        }

        let target_path = cache_manager.get_bundle_path(object_name);

        let url = self.cdn_template.replace("{objectName}", object_name);
        info!("Downloading bundle {} from {}", object_name, url);

        if cancel_token.load(Ordering::SeqCst) {
            return Err(DomainError::importer(
                ErrorCode::ErrCancelled,
                format!("Acquisition of {object_name} cancelled before request"),
            ));
        }

        let part_path = target_path.with_extension("part");
        let response = self.client.get(&url).send().await.map_err(|e| {
            DomainError::importer(
                ErrorCode::ErrIo,
                format!("HTTP request failed for {object_name}: {e}"),
            )
        })?;

        let status = response.status();
        if !status.is_success() {
            return Err(DomainError::importer(
                ErrorCode::ErrIo,
                format!("HTTP error {status} downloading bundle {object_name}"),
            ));
        }

        let total_size = response.content_length().unwrap_or(0);
        if let Some(expected) = expected_size {
            if total_size > 0 && total_size != expected {
                return Err(DomainError::importer(
                    ErrorCode::ErrCorruptedData,
                    format!(
                        "Content-Length mismatch for {object_name}: expected {expected} bytes, server reported {total_size} bytes"
                    ),
                ));
            }
        }

        let mut stream = response.bytes_stream();

        let mut file = File::create(&part_path).map_err(|e| {
            DomainError::io(
                &part_path,
                format!("Failed to create part file {}: {e}", part_path.display()),
            )
        })?;

        let mut downloaded_bytes: u64 = 0;
        let mut hasher = Md5::new();

        while let Some(chunk_result) = stream.next().await {
            if cancel_token.load(Ordering::SeqCst) {
                let _ = fs::remove_file(&part_path);
                return Err(DomainError::importer(
                    ErrorCode::ErrCancelled,
                    format!("Acquisition of {object_name} cancelled by user"),
                ));
            }

            let chunk = chunk_result.map_err(|e| {
                let _ = fs::remove_file(&part_path);
                DomainError::importer(
                    ErrorCode::ErrIo,
                    format!("Error reading stream chunk for {object_name}: {e}"),
                )
            })?;

            file.write_all(&chunk).map_err(|e| {
                let _ = fs::remove_file(&part_path);
                DomainError::io(
                    &part_path,
                    format!("Failed writing to part file {}: {e}", part_path.display()),
                )
            })?;

            hasher.update(&chunk);
            downloaded_bytes += chunk.len() as u64;
            on_progress(downloaded_bytes, total_size);
        }

        file.flush().map_err(|e| {
            let _ = fs::remove_file(&part_path);
            DomainError::io(&part_path, format!("Flush failed: {e}"))
        })?;
        drop(file);

        // Verify actual downloaded bytes against expected size
        if let Some(expected) = expected_size {
            if downloaded_bytes != expected {
                let _ = fs::remove_file(&part_path);
                return Err(DomainError::importer(
                    ErrorCode::ErrCorruptedData,
                    format!(
                        "Size mismatch for {object_name}: expected {expected} bytes, received {downloaded_bytes} bytes"
                    ),
                ));
            }
        }

        // Verify MD5
        let computed_md5 = format!("{:x}", hasher.finalize());
        if !computed_md5.eq_ignore_ascii_case(expected_md5) {
            let _ = fs::remove_file(&part_path);
            return Err(DomainError::importer(
                ErrorCode::ErrCorruptedData,
                format!(
                    "MD5 checksum mismatch for {object_name}: expected {expected_md5}, got {computed_md5}"
                ),
            ));
        }

        // Atomic rename .part -> final bundle path
        fs::rename(&part_path, &target_path).map_err(|e| {
            let _ = fs::remove_file(&part_path);
            DomainError::io(
                &target_path,
                format!("Atomic rename failed for {object_name}: {e}"),
            )
        })?;

        info!(
            "Successfully acquired and verified bundle: {}",
            target_path.display()
        );
        Ok(target_path)
    }

    fn find_cached_bundle(
        &self,
        object_name: &str,
        asset_name: Option<&str>,
        expected_md5: &str,
        cache_manager: &ImporterCacheManager,
    ) -> Option<PathBuf> {
        cache_manager.resolve_cached_bundle(object_name, asset_name, expected_md5)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[tokio::test]
    async fn test_acquire_uses_extensionless_exact_asset_name_in_cache() {
        let dir = tempdir().unwrap();
        let cache = ImporterCacheManager::with_root(dir.path().join("cache"));
        cache.ensure_dirs().unwrap();

        let asset_name = "live2d_mdl_00007-nrml-0008-00";
        let object_name = "DCgZ7m";
        let unityfs_payload = b"UnityFS\0\x00\x00\x00\x086000.3.15f1\0bundle_body";
        let cached_file = cache.bundles_dir().join(asset_name);
        fs::write(&cached_file, unityfs_payload).unwrap();

        // Unreachable CDN verifies acquisition succeeds purely from the extensionless cache file
        let acq = AssetAcquisition::with_cdn_template("http://127.0.0.1:1/{objectName}");
        let cancel = Arc::new(AtomicBool::new(false));
        let mut reported = None;

        let resolved = acq
            .acquire_bundle_for_asset(
                object_name,
                Some(asset_name),
                "a163f3d6bfb59ba534ab1afbd3896120",
                Some(unityfs_payload.len() as u64),
                &cache,
                cancel.clone(),
                |rec, tot| reported = Some((rec, tot)),
            )
            .await
            .unwrap();

        assert_eq!(resolved, cached_file);
        assert_eq!(
            reported,
            Some((unityfs_payload.len() as u64, unityfs_payload.len() as u64))
        );

        // Also supports object_name passed with .bundle when cache file omits .bundle
        let resolved_stripped = acq
            .acquire_bundle(
                &format!("{asset_name}.bundle"),
                "a163f3d6bfb59ba534ab1afbd3896120",
                None,
                &cache,
                cancel,
                |_, _| {},
            )
            .await
            .unwrap();
        assert_eq!(resolved_stripped, cached_file);
    }
}
