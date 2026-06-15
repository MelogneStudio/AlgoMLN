import { useEffect, useRef, useState } from 'react';
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
  const taRef = useRef<HTMLTextAreaElement | null>(null);

  useEffect(() => {
    if (open) {
      setSource(initialSource);
      setError(externalError);
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
        <h2 className={styles.title}>Code your strategy</h2>

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
