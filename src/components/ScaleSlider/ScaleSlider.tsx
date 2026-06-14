import { useEffect, useRef, useState } from 'react';
import {
  MAX_SCALE,
  MIN_SCALE,
  SCALE_STEP,
} from '../../lib/scaling';
import styles from './ScaleSlider.module.css';

interface ScaleSliderProps {
  value: number;
  onChange: (value: number) => void;
  width?: number;
  ariaLabel?: string;
}

/**
 * Continuous slider styled to match the design language. The track is
 * filled up to the thumb position, the rest is empty. The thumb is a
 * draggable pill in `var(--highlight)`.
 */
export function ScaleSlider({
  value,
  onChange,
  width = 360,
  ariaLabel,
}: ScaleSliderProps) {
  const trackRef = useRef<HTMLDivElement | null>(null);
  const [dragging, setDragging] = useState(false);

  const fraction = clamp01((value - MIN_SCALE) / (MAX_SCALE - MIN_SCALE));
  const thumbLeft = fraction * width;

  const setFromClientX = (clientX: number) => {
    const track = trackRef.current;
    if (!track) return;
    const rect = track.getBoundingClientRect();
    const x = clamp01((clientX - rect.left) / rect.width);
    const raw = MIN_SCALE + x * (MAX_SCALE - MIN_SCALE);
    const stepped = Math.round(raw / SCALE_STEP) * SCALE_STEP;
    const clamped = Math.max(MIN_SCALE, Math.min(MAX_SCALE, stepped));
    if (clamped !== value) onChange(parseFloat(clamped.toFixed(2)));
  };

  useEffect(() => {
    if (!dragging) return;
    const onMove = (e: MouseEvent) => {
      e.preventDefault();
      setFromClientX(e.clientX);
    };
    const onUp = () => setDragging(false);
    document.addEventListener('mousemove', onMove);
    document.addEventListener('mouseup', onUp);
    return () => {
      document.removeEventListener('mousemove', onMove);
      document.removeEventListener('mouseup', onUp);
    };
  }, [dragging, value, onChange]);

  const onKeyDown = (e: React.KeyboardEvent<HTMLDivElement>) => {
    if (e.key === 'ArrowLeft' || e.key === 'ArrowDown') {
      e.preventDefault();
      onChange(Math.max(MIN_SCALE, parseFloat((value - SCALE_STEP).toFixed(2))));
    } else if (e.key === 'ArrowRight' || e.key === 'ArrowUp') {
      e.preventDefault();
      onChange(Math.min(MAX_SCALE, parseFloat((value + SCALE_STEP).toFixed(2))));
    } else if (e.key === 'Home') {
      e.preventDefault();
      onChange(MIN_SCALE);
    } else if (e.key === 'End') {
      e.preventDefault();
      onChange(MAX_SCALE);
    }
  };

  return (
    <div
      className={styles.shell}
      style={{ width: `${width}px` }}
    >
      <div
        ref={trackRef}
        className={styles.track}
        onMouseDown={(e) => {
          setDragging(true);
          setFromClientX(e.clientX);
        }}
        role="slider"
        tabIndex={0}
        aria-label={ariaLabel ?? 'interface scale'}
        aria-valuemin={MIN_SCALE}
        aria-valuemax={MAX_SCALE}
        aria-valuenow={value}
        aria-valuetext={`${value.toFixed(2)}x`}
        onKeyDown={onKeyDown}
      >
        <div
          className={styles.fill}
          style={{ width: `${thumbLeft}px` }}
        />
        <div
          className={styles.thumb}
          style={{ left: `${thumbLeft}px` }}
        />
      </div>
    </div>
  );
}

function clamp01(v: number): number {
  if (v < 0) return 0;
  if (v > 1) return 1;
  return v;
}
