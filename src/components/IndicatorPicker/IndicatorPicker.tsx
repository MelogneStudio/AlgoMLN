import { useEffect, useRef, useState } from 'react';
import {
  INDICATOR_DISPLAY,
  INDICATOR_ORDER,
  type IndicatorKind,
} from '../../types/strategy';
import styles from './IndicatorPicker.module.css';

interface IndicatorPickerProps {
  value: IndicatorKind;
  onChange: (value: IndicatorKind) => void;
  width?: number;
}

export function IndicatorPicker({ value, onChange, width = 164 }: IndicatorPickerProps) {
  const [open, setOpen] = useState(false);
  const ref = useRef<HTMLDivElement | null>(null);

  useEffect(() => {
    if (!open) return;
    const handler = (e: MouseEvent) => {
      if (ref.current && !ref.current.contains(e.target as Node)) {
        setOpen(false);
      }
    };
    document.addEventListener('mousedown', handler);
    return () => document.removeEventListener('mousedown', handler);
  }, [open]);

  return (
    <div
      ref={ref}
      className={styles.shell}
      style={{ width: `${width}px` }}
    >
      <button
        type="button"
        className={styles.value}
        onClick={() => setOpen((prev) => !prev)}
        aria-haspopup="listbox"
        aria-expanded={open}
      >
        <span className={styles.label}>{INDICATOR_DISPLAY[value]}</span>
      </button>
      <button
        type="button"
        className={styles.chevron}
        onClick={() => setOpen((prev) => !prev)}
        aria-label="Toggle indicator list"
      >
        <svg viewBox="0 0 12 8" width="12" height="8" fill="none">
          <path
            d="M1 1.5L6 6.5L11 1.5"
            stroke="currentColor"
            strokeWidth="1.5"
            strokeLinecap="round"
            strokeLinejoin="round"
          />
        </svg>
      </button>
      {open && (
        <ul className={styles.menu} role="listbox">
          {INDICATOR_ORDER.map((kind) => (
            <li key={kind}>
              <button
                type="button"
                role="option"
                aria-selected={kind === value}
                className={`${styles.item} ${kind === value ? styles.itemActive : ''}`}
                onClick={() => {
                  onChange(kind);
                  setOpen(false);
                }}
              >
                {INDICATOR_DISPLAY[kind]}
              </button>
            </li>
          ))}
        </ul>
      )}
    </div>
  );
}
