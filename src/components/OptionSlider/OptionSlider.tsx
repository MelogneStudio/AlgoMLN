import styles from './OptionSlider.module.css';

interface OptionSliderProps {
  options: string[];
  selectedIndex: number;
  onChange: (index: number) => void;
  width?: number;
  height?: number;
  ariaLabel?: string;
}

export function OptionSlider({
  options,
  selectedIndex,
  onChange,
  width = 200,
  height = 50,
  ariaLabel,
}: OptionSliderProps) {
  const slotWidth = width / options.length;

  return (
    <div
      className={styles.shell}
      style={{ width: `${width}px`, height: `${height}px` }}
      role="radiogroup"
      aria-label={ariaLabel}
    >
      <div
        className={styles.pill}
        style={{
          width: `calc(${slotWidth}px - 6px)`,
          left: `calc(${selectedIndex * slotWidth}px + 3px)`,
          height: `calc(${height}px - 6px)`,
        }}
      />
      {options.map((opt, idx) => {
        const isActive = idx === selectedIndex;
        return (
          <button
            type="button"
            key={`${opt}-${idx}`}
            className={`${styles.option} ${isActive ? styles.active : ''}`}
            style={{ width: `${slotWidth}px` }}
            role="radio"
            aria-checked={isActive}
            onClick={() => onChange(idx)}
          >
            {opt}
          </button>
        );
      })}
    </div>
  );
}
