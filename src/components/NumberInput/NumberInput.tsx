import { useEffect, useRef, useState } from 'react';
import type { KeyboardEvent } from 'react';
import styles from './NumberInput.module.css';

interface NumberInputProps {
  value: number;
  onChange: (value: number) => void;
  min?: number;
  max?: number;
  width?: number;
  ariaLabel?: string;
}

export function NumberInput({
  value,
  onChange,
  min,
  max,
  width = 200,
  ariaLabel,
}: NumberInputProps) {
  const [editing, setEditing] = useState(false);
  const [draft, setDraft] = useState(String(value));
  const inputRef = useRef<HTMLInputElement | null>(null);

  useEffect(() => {
    if (!editing) setDraft(String(value));
  }, [value, editing]);

  useEffect(() => {
    if (editing && inputRef.current) {
      inputRef.current.focus();
      inputRef.current.select();
    }
  }, [editing]);

  const commit = () => {
    const parsed = parseFloat(draft);
    if (Number.isFinite(parsed)) {
      let clamped = parsed;
      if (typeof min === 'number') clamped = Math.max(clamped, min);
      if (typeof max === 'number') clamped = Math.min(clamped, max);
      if (clamped !== value) onChange(clamped);
    }
    setEditing(false);
  };

  const onKeyDown = (e: KeyboardEvent<HTMLInputElement>) => {
    if (e.key === 'Enter') {
      e.preventDefault();
      commit();
    } else if (e.key === 'Escape') {
      e.preventDefault();
      setDraft(String(value));
      setEditing(false);
    }
  };

  return (
    <div
      className={styles.shell}
      style={{ width: `${width}px` }}
      onDoubleClick={() => setEditing(true)}
    >
      {editing ? (
        <input
          ref={inputRef}
          className={styles.input}
          type="number"
          value={draft}
          min={min}
          max={max}
          aria-label={ariaLabel}
          onChange={(e) => setDraft(e.target.value)}
          onBlur={commit}
          onKeyDown={onKeyDown}
        />
      ) : (
        <button
          type="button"
          className={styles.display}
          aria-label={ariaLabel ?? 'edit number'}
          onClick={() => setEditing(true)}
        >
          {value}
        </button>
      )}
    </div>
  );
}
