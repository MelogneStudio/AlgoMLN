import type { BacktestResult } from '../../types/backtest';
import type { BuilderStrategy } from '../../types/strategy';
import { Button } from '../../components/Button/Button';
import { BacktestPanel } from './components/BacktestPanel';
import { RuleSection } from './components/RuleSection';
import styles from './BuilderScreen.module.css';

interface BuilderScreenProps {
  strategy: BuilderStrategy;
  isAdvancedMode: boolean;
  onEntryChange: (patch: Partial<BuilderStrategy['entry']>) => void;
  onExitChange: (patch: Partial<BuilderStrategy['exit']>) => void;
  onOpenCoder: () => void;
  onOpenUploader: () => void;
  onRunBacktest: () => void;
  onReset: () => void;
  backtest: {
    result: BacktestResult | null;
    isLoading: boolean;
    error: string | null;
  };
  backtestSymbol: string;
  backtestCapital: number;
  onSymbolChange: (symbol: string) => void;
  onCapitalChange: (capital: number) => void;
}

export function BuilderScreen({
  strategy,
  isAdvancedMode,
  onEntryChange,
  onExitChange,
  onOpenCoder,
  onOpenUploader,
  onRunBacktest,
  onReset,
  backtest,
  backtestSymbol,
  backtestCapital,
  onSymbolChange,
  onCapitalChange,
}: BuilderScreenProps) {
  return (
    <div className={styles.shell}>
      <header className={styles.header}>
        <h1 className={styles.title}>Strategy Builder</h1>
        <div className={styles.headerRight}>
          <Button
            variant="code"
            onClick={onOpenCoder}
            icon={
              <svg viewBox="0 0 24 24" width="18" height="18" fill="none">
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
            Open Strategy Coder
          </Button>
        </div>
      </header>

      {isAdvancedMode && (
        <div className={styles.advancedNotice}>
          This strategy uses features the visual builder can&apos;t represent
          (multiple rules, AND/OR, cross conditions). Edit it in the{' '}
          <button
            type="button"
            className={styles.advancedLink}
            onClick={onOpenCoder}
          >
            Strategy Coder
          </button>
          .
        </div>
      )}

      <main className={styles.main}>
        <section className={styles.rulesArea}>
          <div className={styles.rulesRow}>
            <RuleSection
              type="entry"
              rule={strategy.entry}
              onChange={onEntryChange}
            />
          </div>
          <div className={styles.rulesRow}>
            <RuleSection
              type="exit"
              rule={strategy.exit}
              onChange={onExitChange}
            />
          </div>
        </section>

        <section className={styles.actionsRow}>
          <div className={styles.runConfig}>
            <label className={styles.configField}>
              <span>Symbol</span>
              <input
                className={styles.configInput}
                value={backtestSymbol}
                onChange={(e) => onSymbolChange(e.target.value)}
                placeholder="RELIANCE"
              />
            </label>
            <label className={styles.configField}>
              <span>Capital ₹</span>
              <input
                className={styles.configInput}
                type="number"
                value={backtestCapital}
                onChange={(e) => onCapitalChange(parseFloat(e.target.value) || 0)}
                min={1}
              />
            </label>
            <Button
              variant="ghost"
              onClick={onReset}
              icon={
                <svg viewBox="0 0 24 24" width="16" height="16" fill="none">
                  <path
                    d="M3 12a9 9 0 1 0 3-6.7L3 8M3 3v5h5"
                    stroke="currentColor"
                    strokeWidth="1.8"
                    strokeLinecap="round"
                    strokeLinejoin="round"
                  />
                </svg>
              }
            >
              Reset
            </Button>
          </div>
          <div className={styles.runActions}>
            <Button
              variant="primary"
              onClick={onRunBacktest}
              disabled={backtest.isLoading}
              icon={
                <svg viewBox="0 0 24 24" width="18" height="18" fill="none">
                  <path d="M6 4l14 8-14 8V4z" fill="currentColor" />
                </svg>
              }
            >
              {backtest.isLoading ? 'Running...' : 'Backtest'}
            </Button>
            <Button
              variant="primary"
              onClick={onOpenUploader}
              icon={
                <svg viewBox="0 0 24 24" width="18" height="18" fill="none">
                  <path
                    d="M12 4v12M6 10l6-6 6 6M4 20h16"
                    stroke="currentColor"
                    strokeWidth="1.8"
                    strokeLinecap="round"
                    strokeLinejoin="round"
                  />
                </svg>
              }
            >
              Deploy
            </Button>
          </div>
        </section>

        {backtest.error && (
          <div className={styles.error} role="alert">
            {backtest.error}
          </div>
        )}

        <BacktestPanel result={backtest.result} isLoading={backtest.isLoading} />
      </main>
    </div>
  );
}
