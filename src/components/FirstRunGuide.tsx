import React, { useState } from 'react';
import { useI18n } from '../i18n';

interface FirstRunGuideProps {
  isOpen: boolean;
  onClose: (dontShowAgain?: boolean) => void;
}

export const FirstRunGuide: React.FC<FirstRunGuideProps> = ({ isOpen, onClose }) => {
  const { t } = useI18n();
  const [step, setStep] = useState<number>(1);
  const totalSteps = 6;

  if (!isOpen) return null;

  const handleNext = () => {
    if (step < totalSteps) {
      setStep(step + 1);
    } else {
      onClose(true);
    }
  };

  const handlePrev = () => {
    if (step > 1) {
      setStep(step - 1);
    }
  };

  const getStepContent = (s: number) => {
    switch (s) {
      case 1:
        return {
          icon: '✨',
          title: t('onboarding.step1_title'),
          desc: t('onboarding.step1_desc'),
          tip: t('onboarding.step1_tip'),
        };
      case 2:
        return {
          icon: '🎮',
          title: t('onboarding.step2_title'),
          desc: t('onboarding.step2_desc'),
          tip: t('onboarding.step2_tip'),
        };
      case 3:
        return {
          icon: '📦',
          title: t('onboarding.step3_title'),
          desc: t('onboarding.step3_desc'),
          tip: t('onboarding.step3_tip'),
        };
      case 4:
        return {
          icon: '🎭',
          title: t('onboarding.step4_title'),
          desc: t('onboarding.step4_desc'),
          tip: t('onboarding.step4_tip'),
        };
      case 5:
        return {
          icon: '🖥️',
          title: t('onboarding.step5_title'),
          desc: t('onboarding.step5_desc'),
          tip: t('onboarding.step5_tip'),
        };
      case 6:
        return {
          icon: '🖼️',
          title: t('onboarding.step6_title'),
          desc: t('onboarding.step6_desc'),
          tip: t('onboarding.step6_tip'),
        };
      default:
        return { icon: '✨', title: '', desc: '', tip: '' };
    }
  };

  const content = getStepContent(step);

  return (
    <div className="hdm-modal-backdrop" onClick={() => onClose(false)}>
      <div className="hdm-guide-card" onClick={(e) => e.stopPropagation()}>
        <div className="hdm-guide-header">
          <div className="hdm-guide-badge">
            {step} / {totalSteps}
          </div>
          <button className="hdm-guide-close" onClick={() => onClose(false)} title={t('btn.close')}>
            ✕
          </button>
        </div>

        <div className="hdm-guide-body">
          <div className="hdm-guide-big-icon">{content.icon}</div>
          <h2 className="hdm-guide-title">{content.title}</h2>
          <p className="hdm-guide-desc">{content.desc}</p>
          {content.tip && (
            <div className="hdm-guide-tip">
              <span className="hdm-guide-tip-icon">💡</span>
              <span>{content.tip}</span>
            </div>
          )}
        </div>

        <div className="hdm-guide-dots">
          {Array.from({ length: totalSteps }).map((_, i) => (
            <div
              key={i}
              className={`hdm-guide-dot ${step === i + 1 ? 'active' : ''}`}
              onClick={() => setStep(i + 1)}
            />
          ))}
        </div>

        <div className="hdm-guide-footer">
          <div className="hdm-guide-footer-left">
            <button className="btn-text" onClick={() => onClose(true)}>
              {t('onboarding.dont_show_again')}
            </button>
          </div>
          <div className="hdm-guide-footer-right">
            {step > 1 && (
              <button className="btn-secondary" onClick={handlePrev}>
                {t('onboarding.prev')}
              </button>
            )}
            <button className="btn-primary" onClick={handleNext}>
              {step === totalSteps ? t('onboarding.finish') : t('onboarding.next')}
            </button>
          </div>
        </div>
      </div>
    </div>
  );
};
