import { useEffect, useState } from 'react';
import { listStrategies, isTauri } from '../../types/tauri';
import type { DeployedStrategy } from '../../types/strategy';
import { StrategyCard } from './components/StrategyCard';
import styles from './StrategiesScreen.module.css';

interface StrategiesScreenProps {
  refreshKey: number;
  onViewCode: (dsl: string, name: string) => void;
  onChanged: () => void;
}

const DEMO_STRATEGIES: DeployedStrategy[] = [
  {
    id: 'demo-1',
    name: 'RSI Reversal',
    description: 'Buy oversold, sell overbought on 14-period RSI',
    totalPnl: 12_540,
    totalTrades: 87,
    modes: ['paper'],
    status: 'running',
    dslSource: `# Entry
WHEN rsi(14) < 30
BUY 10

# Exit
WHEN rsi(14) > 70
SELL ALL`,
  },
  {
    id: 'demo-2',
    name: 'EMA Crossover',
    description: 'EMA 9 crossing above EMA 21',
    totalPnl: -2_140,
    totalTrades: 41,
    modes: ['paper', 'live'],
    status: 'paused',
    dslSource: `# Entry
WHEN ema(9) > ema(21)
BUY 5

# Exit
WHEN ema(9) < ema(21)
SELL ALL`,
  },
];

export function StrategiesScreen({
  refreshKey,
  onViewCode,
  onChanged,
}: StrategiesScreenProps) {
  const [strategies, setStrategies] = useState<DeployedStrategy[] | null>(null);
  const [error, setError] = useState<string | null>(null);

  const load = async () => {
    setError(null);
    if (!isTauri()) {
      setStrategies(DEMO_STRATEGIES);
      return;
    }
    try {
      const res = await listStrategies();
      setStrategies(res);
    } catch (err) {
      setError(err instanceof Error ? err.message : String(err));
      setStrategies([]);
    }
  };

  useEffect(() => {
    void load();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [refreshKey]);

  return (
    <div className={styles.shell}>
      <header className={styles.header}>
        <h1 className={styles.title}>Deployed Strategies</h1>
        <button
          type="button"
          className={styles.refreshBtn}
          onClick={() => void load()}
          aria-label="Refresh"
        >
          <svg viewBox="0 0 24 24" width="16" height="16" fill="none">
            <path
              d="M3 12a9 9 0 1 0 3-6.7L3 8M3 3v5h5"
              stroke="currentColor"
              strokeWidth="1.8"
              strokeLinecap="round"
              strokeLinejoin="round"
            />
          </svg>
        </button>
      </header>

      {error && <div className={styles.error}>{error}</div>}

      <main className={styles.list}>
        {strategies === null ? (
          <Skeleton />
        ) : strategies.length === 0 ? (
          <div className={styles.empty}>
            <p>No strategies deployed yet.</p>
            <p className={styles.emptyHint}>
              Build one in the Builder, then click Deploy.
            </p>
          </div>
        ) : (
          strategies.map((s) => (
            <StrategyCard
              key={s.id}
              strategy={s}
              onViewCode={onViewCode}
              onChanged={onChanged}
            />
          ))
        )}
      </main>
    </div>
  );
}

function Skeleton() {
  return (
    <>
      <div className={styles.skeleton} />
      <div className={styles.skeleton} />
    </>
  );
}
