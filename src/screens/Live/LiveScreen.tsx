import { useCallback, useEffect, useState } from 'react';
import { getTradeLog, getLiveStatus, pauseLiveStrategy, resumeLiveStrategy, stopLiveStrategy, isTauri } from '../../types/tauri';
import { useLiveStatus } from '../../hooks/useLiveStatus';
import type { LiveStatusWire, TradeLogEntry } from '../../types/live';
import { Button } from '../../components/Button/Button';
import styles from './LiveScreen.module.css';

export function LiveScreen() {
  const { status, error, refresh } = useLiveStatus(5000);
  const [tradeLog, setTradeLog] = useState<TradeLogEntry[]>([]);
  const [logLoading, setLogLoading] = useState(false);
  const [logError, setLogError] = useState<string | null>(null);

  const loadTradeLog = useCallback(async () => {
    setLogLoading(true);
    setLogError(null);
    try {
      const log = await getTradeLog();
      setTradeLog(log);
    } catch (e) {
      setLogError(String(e));
    } finally {
      setLogLoading(false);
    }
  }, []);

  useEffect(() => {
    loadTradeLog();
  }, [loadTradeLog]);

  const handlePause = useCallback(async () => {
    try {
      await pauseLiveStrategy();
      refresh();
    } catch (e) {
      console.error('Failed to pause:', e);
    }
  }, [refresh]);

  const handleResume = useCallback(async () => {
    try {
      await resumeLiveStrategy();
      refresh();
    } catch (e) {
      console.error('Failed to resume:', e);
    }
  }, [refresh]);

  const handleStop = useCallback(async () => {
    try {
      await stopLiveStrategy();
      refresh();
      loadTradeLog();
    } catch (e) {
      console.error('Failed to stop:', e);
    }
  }, [refresh, loadTradeLog]);

  if (error) {
    return (
      <div className={styles.shell}>
        <div className={styles.errorCard}>
          <p>Failed to load live status: {error}</p>
          <Button variant="ghost" onClick={refresh}>Retry</Button>
        </div>
      </div>
    );
  }

  const noSession = status === null;
  const isRunning = status?.status === 'Running';
  const isPaused = status?.status === 'Paused';
  const isStarting = status?.status === 'Starting';
  const isFailed = status?.status === 'Failed';
  const isStopped = status?.status === 'Stopped';

  return (
    <div className={styles.shell}>
      {/* Section 1: Status Card */}
      <section className={styles.card}>
        <header className={styles.cardHeader}>
          <h2 className={styles.cardTitle}>
            Live Trading
            {isRunning && <span className={styles.pulseDot} aria-label="Running" />}
          </h2>
        </header>
        <div className={styles.cardBody}>
          {noSession ? (
            <p className={styles.emptyText}>No live strategy running.</p>
          ) : (
            <>
              <div className={styles.statusMain}>
                <div className={styles.statusLeft}>
                  <h3 className={styles.strategyName}>{status!.strategyName}</h3>
                  <span className={styles.symbolChip}>{status!.symbol}</span>
                </div>
                <div className={styles.statusBadges}>
                  <span className={`${styles.statusBadge} ${styles[status!.status.toLowerCase()]}`}>
                    {status!.status}
                  </span>
                  {status!.lossTrackingStale && (
                    <span className={styles.staleBadge}>⚠ Loss tracking stale</span>
                  )}
                </div>
              </div>

              <div className={styles.statusMeta}>
                <span className={styles.metaItem}>
                  Started {formatTime(status!.startTime)}
                </span>
                <span
                  className={`${styles.metaItem} ${styles.realizedLoss}`}
                  style={{ color: status!.realizedLoss > 0 ? '#c85a54' : 'var(--text-dim)' }}
                >
                  Session loss: ₹{status!.realizedLoss.toFixed(2)}
                </span>
              </div>

              {isFailed && status!.failReason && (
                <div className={styles.failReason}>{status!.failReason}</div>
              )}
            </>
          )}
        </div>
      </section>

      {/* Section 2: Positions Card */}
      {status && (isRunning || isPaused) && (
        <section className={styles.card}>
          <header className={styles.cardHeader}>
            <h2 className={styles.cardTitle}>Open Positions</h2>
            <span className={styles.countBadge}>{status.positionCount}</span>
          </header>
          <div className={styles.cardBody}>
            <p className={styles.positionsNote}>
              Detailed positions view coming in Phase 8.
            </p>
          </div>
        </section>
      )}

      {/* Section 3: Controls Row */}
      {status && !noSession && (
        <section className={styles.controlsCard}>
          <div className={styles.controlsRow}>
            <Button
              variant="ghost"
              onClick={handlePause}
              disabled={!isRunning}
              title={isRunning ? '' : 'Only available when running'}
            >
              <svg viewBox="0 0 24 24" width="16" height="16" fill="currentColor" className={styles.controlIcon}>
                <rect x="6" y="5" width="4" height="14" rx="1" />
                <rect x="14" y="5" width="4" height="14" rx="1" />
              </svg>
              Pause
            </Button>
            <span className={styles.controlSubtitle}>
              Pausing stops new entries only. Existing stops and risk rules continue to run.
            </span>

            <Button
              variant="ghost"
              onClick={handleResume}
              disabled={!isPaused}
              title={isPaused ? '' : 'Only available when paused'}
            >
              <svg viewBox="0 0 24 24" width="16" height="16" fill="currentColor" className={styles.controlIcon}>
                <path d="M6 4l14 8-14 8V4z" />
              </svg>
              Resume
            </Button>

            <Button
              variant="ghost"
              onClick={handleStop}
              disabled={isStarting}
              className={styles.stopBtn}
            >
              <svg viewBox="0 0 24 24" width="16" height="16" fill="currentColor" className={styles.controlIcon}>
                <rect x="6" y="6" width="12" height="12" rx="2" />
              </svg>
              Stop
            </Button>
          </div>
        </section>
      )}

      {/* Section 4: Trade Log Card */}
      <section className={styles.card}>
        <header className={styles.cardHeader}>
          <h2 className={styles.cardTitle}>Trade Log</h2>
          <span className={styles.countBadge}>{tradeLog.length}</span>
        </header>
        <div className={styles.cardBody}>
          {logLoading ? (
            <p className={styles.emptyText}>Loading…</p>
          ) : logError ? (
            <div className={styles.logError}>
              Failed to load trade log: {logError}
              <Button variant="ghost" onClick={loadTradeLog} style={{ marginLeft: 12 }}>
                ↻ Refresh
              </Button>
            </div>
          ) : tradeLog.length === 0 ? (
            <p className={styles.emptyText}>No live trades recorded yet.</p>
          ) : (
            <div className={styles.logTableWrapper}>
              <table className={styles.logTable}>
                <thead>
                  <tr>
                    <th>Time</th>
                    <th>Strategy</th>
                    <th>Symbol</th>
                    <th>Side</th>
                    <th>Qty</th>
                    <th>Price</th>
                    <th>Status</th>
                    <th>Order ID</th>
                    <th>Notes</th>
                  </tr>
                </thead>
                <tbody>
                  {tradeLog.map((entry) => (
                    <tr key={entry.id}>
                      <td className={styles.logTime}>{formatTime(entry.timestamp)}</td>
                      <td className={styles.logStrategy}>{entry.strategyName}</td>
                      <td className={styles.logSymbol}>{entry.symbol}</td>
                      <td className={entry.side === 'BUY' ? styles.logBuy : styles.logSell}>
                        {entry.side}
                      </td>
                      <td className={styles.logQty}>{entry.quantity}</td>
                      <td className={entry.orderStatus !== 'TRADED' ? styles.logPriceDim : styles.logPrice}>
                        {entry.orderStatus !== 'TRADED' ? '—' : entry.price.toFixed(2)}
                      </td>
                      <td>
                        <span className={`${styles.logStatus} ${styles[statusClass(entry.orderStatus)]}`}>
                          {formatStatus(entry.orderStatus)}
                        </span>
                      </td>
                      <td className={styles.logOrderId}>{shortId(entry.orderId)}</td>
                      <td className={`${styles.logNotes} ${notesClass(entry.notes)}`}>
                        {entry.notes || '—'}
                      </td>
                    </tr>
                  ))}
                </tbody>
              </table>
            </div>
          )}
          <div className={styles.logFooter}>
            <Button variant="ghost" onClick={loadTradeLog} disabled={logLoading}>
              ↻ Refresh
            </Button>
          </div>
        </div>
      </section>
    </div>
  );
}

function formatTime(iso: string): string {
  const d = new Date(iso);
  return d.toLocaleTimeString('en-IN', { hour: '2-digit', minute: '2-digit', second: '2-digit', hour12: false });
}

function formatStatus(s: string): string {
  const map: Record<string, string> = {
    TRADED: 'TRADED',
    PENDING: 'PENDING',
    TRANSIT: 'TRANSIT',
    REJECTED: 'REJECTED',
    CANCELLED: 'CANCELLED',
    EXPIRED: 'EXPIRED',
  };
  return map[s] || s;
}

function statusClass(s: string): string {
  if (s === 'TRADED') return 'statusTraded';
  if (s === 'PENDING' || s === 'TRANSIT') return 'statusPending';
  return 'statusRejected';
}

function notesClass(notes: string): string {
  if (!notes) return 'notesEmpty';
  if (notes === 'stop_loss' || notes === 'risk_breach') return 'notesRisk';
  if (notes === 'take_profit') return 'notesProfit';
  return '';
}

function shortId(id: string): string {
  return id.length > 12 ? id.slice(0, 12) + '…' : id;
}