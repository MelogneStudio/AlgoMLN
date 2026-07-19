import { useEffect, useMemo, useRef, useState } from 'react';
import type { KeyboardEvent } from 'react';
import { Button } from '../../components/Button/Button';
import styles from './StrategyCoderScreen.module.css';

interface StrategyCoderScreenProps {
  open: boolean;
  initialSource: string;
  onClose: () => void;
  onSave: (source: string) => void;
  readOnly?: boolean;
  error?: string | null;
}

const TRADE_IN_HINT = `# TRADE_IN syntax (optional — place before first WHEN):
#
#   TRADE_IN NIFTY_50          — all 50 large-cap constituents
#   TRADE_IN NIFTY_BANK        — banking sector
#   TRADE_IN RELIANCE, INFY    — explicit symbol list
#
# Supported indices:
#   NIFTY_50  NIFTY_NEXT_50  NIFTY_100  NIFTY_200  NIFTY_500
#   NIFTY_MIDCAP_50  NIFTY_MIDCAP_100  NIFTY_MIDCAP_150
#   NIFTY_SMALLCAP_50  NIFTY_SMALLCAP_100  NIFTY_SMALLCAP_250
#   NIFTY_BANK  NIFTY_IT  NIFTY_PHARMA  NIFTY_AUTO  NIFTY_FMCG
#   NIFTY_METAL  NIFTY_REALTY  NIFTY_ENERGY  NIFTY_INFRA
#   NIFTY_PSU_BANK  NIFTY_FINANCIAL_SERVICES
#
# Multi-symbol strategies: paper/live only (backtest not yet supported).`;

export function StrategyCoderScreen({
  open,
  initialSource,
  onClose,
  onSave,
  readOnly = false,
  error: externalError = null,
}: StrategyCoderScreenProps) {
  const [source, setSource] = useState(initialSource);
  const [error, setError] = useState<string | null>(externalError);
  const [showHint, setShowHint] = useState(false);
  const taRef = useRef<HTMLTextAreaElement | null>(null);

  const tradeInLabel = useMemo(() => {
    const m = source.match(/^\s*TRADE_IN\s+(.+)$/im);
    return m ? m[1].trim() : null;
  }, [source]);

  useEffect(() => {
    if (open) {
      setSource(initialSource);
      setError(externalError);
      setShowHint(false);
    }
  }, [open, initialSource, externalError]);

  if (!open) return null;

  const handleKeyDown = (e: KeyboardEvent<HTMLTextAreaElement>) => {
    if (e.key === 'Tab' && !readOnly) {
      e.preventDefault();
      const ta = e.currentTarget;
      const start = ta.selectionStart;
      const end = ta.selectionEnd;
      const next = source.slice(0, start) + '  ' + source.slice(end);
      setSource(next);
      requestAnimationFrame(() => {
        if (taRef.current) {
          taRef.current.selectionStart = taRef.current.selectionEnd = start + 2;
        }
      });
    } else if (e.key === 'Escape') {
      e.preventDefault();
      onClose();
    }
  };

  const handleDone = () => {
    if (readOnly) {
      onClose();
      return;
    }
    if (!source.trim()) {
      setError('Strategy source is empty.');
      return;
    }
    onSave(source);
  };

  return (
    <div className={styles.overlay} role="dialog" aria-modal="true">
      <div className={styles.card}>
        <div className={styles.titleRow}>
          <h2 className={styles.title}>Code your strategy</h2>
          {tradeInLabel && (
            <span className={styles.tradeInChip}>TRADE_IN: {tradeInLabel}</span>
          )}
        </div>

        <div className={styles.toolbar}>
          <button
            type="button"
            className={styles.hintToggle}
            onClick={() => setShowHint((v) => !v)}
            aria-expanded={showHint}
          >
            {showHint ? 'Hide TRADE_IN reference' : 'Show TRADE_IN reference'}
          </button>
        </div>

        {showHint && (
          <pre className={styles.hintBlock} aria-label="TRADE_IN syntax reference">
            {TRADE_IN_HINT}
          </pre>
        )}

        <div className={styles.editorWrap}>
          <div className={styles.fileTab}>
            <span>strategy.algomln</span>
            <svg viewBox="0 0 16 16" width="12" height="12" fill="none">
              <path
                d="M11 2l3 3-7 7-3 1 1-3 6-8z"
                stroke="currentColor"
                strokeWidth="1.4"
                strokeLinecap="round"
                strokeLinejoin="round"
              />
            </svg>
          </div>
          <textarea
            ref={taRef}
            className={styles.editor}
            value={source}
            onChange={(e) => {
              setSource(e.target.value);
              if (error) setError(null);
            }}
            onKeyDown={handleKeyDown}
            spellCheck={false}
            readOnly={readOnly}
            aria-label="strategy source"
          />
        </div>

        {error && (
          <div className={styles.error} role="alert">
            {error}
          </div>
        )}

        <div className={styles.actions}>
          <Button
            variant="ghost"
            onClick={onClose}
            icon={
              <svg viewBox="0 0 24 24" width="16" height="16" fill="none">
                <path
                  d="M6 6l12 12M6 18L18 6"
                  stroke="currentColor"
                  strokeWidth="1.8"
                  strokeLinecap="round"
                />
              </svg>
            }
          >
            Cancel
          </Button>
          <Button
            variant="primary"
            onClick={handleDone}
            icon={
              <svg viewBox="0 0 24 24" width="16" height="16" fill="none">
                <path
                  d="M5 12l5 5L20 7"
                  stroke="currentColor"
                  strokeWidth="2"
                  strokeLinecap="round"
                  strokeLinejoin="round"
                />
              </svg>
            }
          >
            {readOnly ? 'Close' : 'Done'}
          </Button>
        </div>
      </div>
    </div>
  );
}
