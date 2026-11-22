use md5::{Digest, Md5};
use serde::{Deserialize, Serialize};
use std::ffi::OsStr;
use std::fs;
use std::path::{Path, PathBuf};
use tracing::info;

use crate::domain::error::DomainError;
use crate::domain::octo::create_hash_mask;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CacheStats {
    pub cached_bundles_count: usize,
    pub total_bytes: u64,
    pub cache_directory: String,
}

pub struct ImporterCacheManager {
    cache_root: PathBuf,
}

impl Default for ImporterCacheManager {
    fn default() -> Self {
        Self::new()
    }
}

impl ImporterCacheManager {
    pub fn new() -> Self {
        let base = std::env::var_os("LOCALAPPDATA")
            .map(PathBuf::from)
            .unwrap_or_else(std::env::temp_dir)
            .join("HoloDoriLive2DManager")
            .join("cache");
        Self { cache_root: base }
    }

    pub fn with_root(root: impl AsRef<Path>) -> Self {
        Self {
            cache_root: root.as_ref().to_path_buf(),
        }
    }

    pub fn cache_root(&self) -> &Path {
        &self.cache_root
    }

    pub fn bundles_dir(&self) -> PathBuf {
        self.cache_root.join("bundles")
    }

    pub fn ensure_dirs(&self) -> Result<(), DomainError> {
        let dir = self.bundles_dir();
        fs::create_dir_all(&dir).map_err(|e| {
            DomainError::io(
                &dir,
                format!("Failed to create bundle cache directory: {e}"),
            )
        })
    }

    pub fn get_bundle_path(&self, object_name: &str) -> PathBuf {
        self.bundles_dir().join(object_name)
    }

    pub fn is_bundle_cached(&self, object_name: &str, expected_md5: &str) -> bool {
        self.resolve_cached_bundle(object_name, None, expected_md5)
            .is_some()
    }

    pub fn is_asset_bundle_cached(
        &self,
        object_name: &str,
        asset_name: &str,
        expected_md5: &str,
    ) -> bool {
        self.resolve_cached_bundle(object_name, Some(asset_name), expected_md5)
            .is_some()
    }

    /// Searches `cache/bundles/` (flat) for a valid cached bundle matching `object_name`
    /// or an exact match to `asset_name` without `.bundle` (as well as `<asset_name>.bundle`).
    pub fn resolve_cached_bundle(
        &self,
        object_name: &str,
        asset_name: Option<&str>,
        expected_md5: &str,
    ) -> Option<PathBuf> {
        let clean_asset_name = asset_name
            .map(|a| a.strip_suffix(".bundle").unwrap_or(a))
            .filter(|a| !a.is_empty());

        // 1. Direct object_name lookup in bundles/
        if let Some(path) = self.find_exact_file_in_bundles(object_name) {
            let remask_key = clean_asset_name
                .or_else(|| Some(object_name.strip_suffix(".bundle").unwrap_or(object_name)));
            if Self::validate_cached_file(&path, remask_key, expected_md5) {
                return Some(path);
            }
        }

        // 2. If object_name ends with .bundle, check exact stem without .bundle
        if let Some(obj_stem) = object_name.strip_suffix(".bundle") {
            let matches_asset = clean_asset_name.map_or(true, |a| a == obj_stem);
            if matches_asset {
                if let Some(path) = self.find_exact_file_in_bundles(obj_stem) {
                    if Self::validate_cached_file(&path, Some(obj_stem), expected_md5) {
                        return Some(path);
                    }
                }
            }
        }

        // 3. Exact match to asset_name without .bundle in bundles/ (plus <asset_name>.bundle)
        if let Some(exact_asset) = clean_asset_name {
            if exact_asset != object_name {
                if let Some(path) = self.find_exact_file_in_bundles(exact_asset) {
                    if Self::validate_cached_file(&path, Some(exact_asset), expected_md5) {
                        return Some(path);
                    }
                }
            }

            let asset_with_ext = format!("{exact_asset}.bundle");
            if asset_with_ext != object_name {
                if let Some(path) = self.find_exact_file_in_bundles(&asset_with_ext) {
                    if Self::validate_cached_file(&path, Some(exact_asset), expected_md5) {
                        return Some(path);
                    }
                }
            }
        }

        None
    }

    fn is_safe_single_filename(name: &str) -> bool {
        if name.is_empty() || name == "." || name == ".." {
            return false;
        }
        !name.contains(['/', '\\', ':', '\0'])
    }

    fn find_exact_file_in_bundles(&self, expected_name: &str) -> Option<PathBuf> {
        if !Self::is_safe_single_filename(expected_name) {
            return None;
        }
        let bundles_dir = self.bundles_dir();
        let candidate = bundles_dir.join(expected_name);
        if !candidate.is_file() {
            return None;
        }

        let entries = fs::read_dir(&bundles_dir).ok()?;
        let expected_os = OsStr::new(expected_name);
        for entry in entries.flatten() {
            if entry.file_name() == expected_os {
                if entry.metadata().map(|m| m.is_file()).unwrap_or(false) {
                    return Some(entry.path());
                }
            }
        }
        None
    }

    fn validate_cached_file(path: &Path, asset_name: Option<&str>, expected_md5: &str) -> bool {
        let Ok(bytes) = fs::read(path) else {
            return false;
        };
        if bytes.is_empty() {
            return false;
        }

        if !expected_md5.is_empty() {
            let mut hasher = Md5::new();
            hasher.update(&bytes);
            let actual_md5 = format!("{:x}", hasher.finalize());
            if actual_md5.eq_ignore_ascii_case(expected_md5) {
                return true;
            }

            if bytes.starts_with(b"Unity") {
                if let Some(name) = asset_name {
                    let mask = create_hash_mask(name);
                    if !mask.is_empty() {
                        let limit = 256.min(bytes.len());
                        let mut remasked_prefix = bytes[..limit].to_vec();
                        for (i, b) in remasked_prefix.iter_mut().enumerate() {
                            *b ^= mask[i % mask.len()];
                        }
                        let mut remask_hasher = Md5::new();
                        remask_hasher.update(&remasked_prefix);
                        remask_hasher.update(&bytes[limit..]);
                        let remasked_md5 = format!("{:x}", remask_hasher.finalize());
                        if remasked_md5.eq_ignore_ascii_case(expected_md5) {
                            return true;
                        }
                    }
                }
            }
        }

        bytes.starts_with(b"UnityFS\0") && bytes.len() >= 16
    }

    pub fn get_stats(&self) -> CacheStats {
        let bundles = self.bundles_dir();
        let mut count = 0;
        let mut total_bytes = 0;

        if let Ok(entries) = fs::read_dir(&bundles) {
            for entry in entries.flatten() {
                if let Ok(meta) = entry.metadata() {
                    if meta.is_file() {
                        count += 1;
                        total_bytes += meta.len();
                    }
                }
            }
        }

        CacheStats {
            cached_bundles_count: count,
            total_bytes,
            cache_directory: self.cache_root.to_string_lossy().to_string(),
        }
    }

    pub fn clear_cache(&self) -> Result<u64, DomainError> {
        let stats = self.get_stats();
        let bundles = self.bundles_dir();
        if bundles.exists() {
            fs::remove_dir_all(&bundles).map_err(|e| {
                DomainError::io(
                    &bundles,
                    format!("Failed to clear bundle cache directory: {e}"),
                )
            })?;
        }
        self.ensure_dirs()?;
        info!("Cleared {} bytes from bundle cache", stats.total_bytes);
        Ok(stats.total_bytes)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn test_cache_manager() {
        let dir = tempdir().unwrap();
        let cache = ImporterCacheManager::with_root(dir.path().join("cache"));
        cache.ensure_dirs().unwrap();

        let obj_name = "test_bundle_123";
        let content = b"bundle content";
        let mut hasher = Md5::new();
        hasher.update(content);
        let expected_md5 = format!("{:x}", hasher.finalize());

        assert!(!cache.is_bundle_cached(obj_name, &expected_md5));

        let bundle_path = cache.get_bundle_path(obj_name);
        fs::write(&bundle_path, content).unwrap();

        assert!(cache.is_bundle_cached(obj_name, &expected_md5));
        assert!(!cache.is_bundle_cached(obj_name, "wrongmd5"));

        let stats = cache.get_stats();
        assert_eq!(stats.cached_bundles_count, 1);
        assert_eq!(stats.total_bytes, content.len() as u64);

        let reclaimed = cache.clear_cache().unwrap();
        assert_eq!(reclaimed, content.len() as u64);
        assert_eq!(cache.get_stats().cached_bundles_count, 0);
    }

    #[test]
    fn test_resolve_extensionless_asset_name_and_unityfs() {
        let dir = tempdir().unwrap();
        let cache = ImporterCacheManager::with_root(dir.path().join("cache"));
        cache.ensure_dirs().unwrap();

        let obj_name = "DCgZ7m";
        let asset_name = "live2d_mdl_00007-nrml-0008-00";

        // 1. Raw bytes matching catalog MD5 stored under exact asset_name (no .bundle)
        let raw_bytes = b"obfuscated_octo_bundle_payload_bytes";
        let mut hasher = Md5::new();
        hasher.update(raw_bytes);
        let expected_md5 = format!("{:x}", hasher.finalize());

        let extless_path = cache.bundles_dir().join(asset_name);
        fs::write(&extless_path, raw_bytes).unwrap();

        assert_eq!(
            cache.resolve_cached_bundle(obj_name, Some(asset_name), &expected_md5),
            Some(extless_path.clone())
        );
        // Partial / non-exact asset_name must not match
        assert_eq!(
            cache.resolve_cached_bundle(obj_name, Some("live2d_mdl_00007-nrml-0008"), &expected_md5),
            None
        );

        // 2. Pre-unmasked UnityFS header stored under exact asset_name (no .bundle)
        let unityfs_bytes = b"UnityFS\0\x00\x00\x00\x086000.3.15f1\0payload";
        fs::write(&extless_path, unityfs_bytes).unwrap();
        assert_eq!(
            cache.resolve_cached_bundle(obj_name, Some(asset_name), "different_catalog_md5"),
            Some(extless_path.clone())
        );

        // 3. Corrupted non-UnityFS bytes with mismatched MD5 must be rejected
        fs::write(&extless_path, b"corrupted_non_unity_data").unwrap();
        assert_eq!(
            cache.resolve_cached_bundle(obj_name, Some(asset_name), &expected_md5),
            None
        );
    }
}
