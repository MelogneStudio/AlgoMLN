import type { BacktestResult } from '../../../types/backtest';
import styles from './BacktestPanel.module.css';

interface BacktestPanelProps {
  result: BacktestResult | null;
  isLoading: boolean;
}

function formatCurrency(value: number): string {
  const sign = value < 0 ? '-' : '';
  const abs = Math.abs(value);
  return `${sign}₹${abs.toLocaleString('en-IN', {
    maximumFractionDigits: 0,
  })}`;
}

function formatPnl(value: number): string {
  if (value > 0) return `+${formatCurrency(value)}`;
  if (value < 0) return `-${formatCurrency(Math.abs(value))}`;
  return '₹0';
}

function formatPct(value: number): string {
  return `${value.toFixed(2)}%`;
}

export function BacktestPanel({ result, isLoading }: BacktestPanelProps) {
  const hasData = result !== null && result.tradeHistory.length > 0;
  const summary = result?.summary;

  return (
    <section className={styles.shell} aria-label="Backtest results">
      <div className={styles.overview}>
        <div className={styles.row}>
          <span className={styles.label}>Net PnL</span>
          <span
            className={`${styles.value} ${
              summary && summary.totalRealizedPnl > 0
                ? styles.positive
                : summary && summary.totalRealizedPnl < 0
                  ? styles.negative
                  : ''
            }`}
          >
            {isLoading
              ? '...'
              : summary
                ? formatPnl(summary.totalRealizedPnl)
                : '—'}
          </span>
        </div>
        <div className={styles.row}>
          <span className={styles.label}>Return</span>
          <span className={styles.value}>
            {isLoading
              ? '...'
              : summary
                ? formatPct(summary.totalReturnPct)
                : '—'}
          </span>
        </div>
        <div className={styles.row}>
          <span className={styles.label}>Max Loss</span>
          <span className={styles.value}>
            {isLoading
              ? '...'
              : summary
                ? formatCurrency(summary.maxDrawdown)
                : '—'}
          </span>
        </div>
        <div className={styles.row}>
          <span className={styles.label}>Max Profit</span>
          <span className={styles.value}>
            {isLoading
              ? '...'
              : summary
                ? formatCurrency(summary.largestWin)
                : '—'}
          </span>
        </div>
        <div className={styles.row}>
          <span className={styles.label}>Win Rate</span>
          <span className={styles.value}>
            {isLoading
              ? '...'
              : summary
                ? formatPct(summary.winRatePct)
                : '—'}
          </span>
        </div>
        <div className={styles.row}>
          <span className={styles.label}>Total Trades</span>
          <span className={styles.value}>
            {isLoading ? '...' : summary ? summary.totalTrades : '—'}
          </span>
        </div>
      </div>

      <div className={styles.results}>
        {!result ? (
          <div className={styles.empty}>Oops! No data found</div>
        ) : !hasData ? (
          <div className={styles.empty}>
            Backtest finished with no trades. Try a different strategy.
          </div>
        ) : (
          <div className={styles.tableWrap}>
            <table className={styles.table}>
              <thead>
                <tr>
                  <th>Time</th>
                  <th>Side</th>
                  <th>Symbol</th>
                  <th>Qty</th>
                  <th>Price</th>
                  <th>PnL</th>
                </tr>
              </thead>
              <tbody>
                {result.tradeHistory.map((t) => (
                  <tr key={t.id}>
                    <td>{t.timestamp}</td>
                    <td
                      className={
                        t.side === 'buy' ? styles.buy : styles.sell
                      }
                    >
                      {t.side.toUpperCase()}
                    </td>
                    <td>{t.symbol}</td>
                    <td>{t.quantity}</td>
                    <td>{formatCurrency(t.price)}</td>
                    <td
                      className={
                        t.pnl === null
                          ? ''
                          : t.pnl > 0
                            ? styles.positive
                            : t.pnl < 0
                              ? styles.negative
                              : ''
                      }
                    >
                      {t.pnl === null ? '—' : formatPnl(t.pnl)}
                    </td>
                  </tr>
                ))}
              </tbody>
            </table>
          </div>
        )}
      </div>
    </section>
  );
}
