import { useState } from 'react';
import { Button } from '../../../components/Button/Button';
import type { DeployedStrategy } from '../../../types/strategy';
import { setStrategyStatus } from '../../../types/tauri';
import styles from './StrategyCard.module.css';

interface StrategyCardProps {
  strategy: DeployedStrategy;
  onViewCode: (dsl: string, name: string) => void;
  onChanged: () => void;
}

function formatCurrency(value: number): string {
  const sign = value < 0 ? '-' : '';
  const abs = Math.abs(value);
  return `${sign}₹${abs.toLocaleString('en-IN', { maximumFractionDigits: 0 })}`;
}

export function StrategyCard({ strategy, onViewCode, onChanged }: StrategyCardProps) {
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const handleToggle = async () => {
    setBusy(true);
    setError(null);
    try {
      await setStrategyStatus(
        strategy.id,
        strategy.status === 'running' ? 'paused' : 'running'
      );
      onChanged();
    } catch (err) {
      setError(err instanceof Error ? err.message : String(err));
    } finally {
      setBusy(false);
    }
  };

  return (
    <article className={styles.card}>
      <header className={styles.topRow}>
        <div className={styles.titleGroup}>
          <h3 className={styles.name}>{strategy.name}</h3>
          <button
            type="button"
            className={styles.editBtn}
            aria-label="Edit name"
            onClick={() => onViewCode(strategy.dslSource, strategy.name)}
          >
            <svg viewBox="0 0 16 16" width="12" height="12" fill="none">
              <path
                d="M11 2l3 3-7 7-3 1 1-3 6-8z"
                stroke="currentColor"
                strokeWidth="1.4"
                strokeLinecap="round"
                strokeLinejoin="round"
              />
            </svg>
          </button>
        </div>
        <p className={styles.description}>
          {strategy.description || 'No description'}
        </p>
        <div className={styles.badges}>
          {strategy.modes.map((mode) => (
            <span
              key={mode}
              className={styles.badge}
              data-mode={mode}
            >
              {mode}
            </span>
          ))}
        </div>
      </header>

      <div className={styles.middleRow}>
        <div className={styles.metric}>
          <span>Total PnL:</span>
          <span
            className={
              strategy.totalPnl > 0
                ? styles.positive
                : strategy.totalPnl < 0
                  ? styles.negative
                  : ''
            }
          >
            {formatCurrency(strategy.totalPnl)}
          </span>
        </div>
        <div className={styles.metric}>
          <span>Total Trades:</span>
          <span className={styles.yellow}>{strategy.totalTrades}</span>
        </div>
      </div>

      <footer className={styles.bottomRow}>
        <Button
          variant="primary"
          onClick={handleToggle}
          disabled={busy}
          icon={
            strategy.status === 'running' ? (
              <svg viewBox="0 0 24 24" width="14" height="14" fill="currentColor">
                <rect x="6" y="5" width="4" height="14" rx="1" />
                <rect x="14" y="5" width="4" height="14" rx="1" />
              </svg>
            ) : (
              <svg viewBox="0 0 24 24" width="14" height="14" fill="currentColor">
                <path d="M6 4l14 8-14 8V4z" />
              </svg>
            )
          }
        >
          {strategy.status === 'running' ? 'Pause' : 'Resume'}
        </Button>
        <Button
          variant="code"
          onClick={() => onViewCode(strategy.dslSource, strategy.name)}
          icon={
            <svg viewBox="0 0 24 24" width="14" height="14" fill="none">
              <path
                d="M9 6l-6 6 6 6M15 6l6 6-6 6"
                stroke="currentColor"
                strokeWidth="1.8"
                strokeLinecap="round"
                strokeLinejoin="round"
              />
            </svg>
          }
        >
          View Code
        </Button>
      </footer>
      {error && <div className={styles.error}>{error}</div>}
    </article>
  );
}
