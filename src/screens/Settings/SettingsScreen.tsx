import { useEffect, useState } from 'react';
import {
  CAPITAL_STORAGE_KEY,
  loadSavedCapital,
  saveCapital,
} from '../../lib/scaling';
import { listIndices, refreshIndices } from '../../types/tauri';
import type { IndexInfo } from '../../types/tauri';
import { Button } from '../../components/Button/Button';
import styles from './SettingsScreen.module.css';

export function SettingsScreen() {
  const [capital, setCapital] = useState<number>(() => loadSavedCapital());
  const [capitalDraft, setCapitalDraft] = useState<string>(String(capital));

  const [indices, setIndices] = useState<IndexInfo[]>([]);
  const [isRefreshing, setIsRefreshing] = useState(false);
  const [refreshMsg, setRefreshMsg] = useState<string | null>(null);
  const [refreshMsgKind, setRefreshMsgKind] = useState<'success' | 'error'>('success');

  useEffect(() => {
    let cancelled = false;
    listIndices()
      .then((list) => {
        if (!cancelled) setIndices(list);
      })
      .catch(() => {
        if (!cancelled) setIndices([]);
      });
    return () => {
      cancelled = true;
    };
  }, []);

  const handleRefresh = async () => {
    setIsRefreshing(true);
    setRefreshMsg(null);
    try {
      const result = await refreshIndices();
      const ok = result.refreshed.length;
      const fail = result.failed.length;
      setRefreshMsgKind(fail === 0 ? 'success' : 'error');
      setRefreshMsg(
        fail === 0
          ? `All ${ok} indices refreshed.`
          : `${ok} refreshed, ${fail} failed.`
      );
      listIndices().then(setIndices).catch(() => {});
    } catch (e) {
      setRefreshMsgKind('error');
      setRefreshMsg('Refresh failed — check network connection.');
    } finally {
      setIsRefreshing(false);
    }
  };

  return (
    <div className={styles.shell}>
      <header className={styles.header}>
        <h1 className={styles.title}>Settings</h1>
      </header>

      <main className={styles.grid}>
        <article className={styles.card}>
          <h2 className={styles.cardLabel}>Connected Broker</h2>
          <p className={styles.cardValue}>Dhan (API Key set)</p>
          <p className={styles.cardSubtext}>Upstox support coming soon</p>
        </article>

        <article className={styles.card}>
          <h2 className={styles.cardLabel}>Default Backtest Capital</h2>
          <div className={styles.capitalRow}>
            <span className={styles.rupee}>₹</span>
            <input
              className={styles.capitalInput}
              type="number"
              value={capitalDraft}
              min={1}
              onChange={(e) => setCapitalDraft(e.target.value)}
              onBlur={() => {
                const parsed = parseFloat(capitalDraft);
                if (Number.isFinite(parsed) && parsed > 0) {
                  setCapital(parsed);
                  saveCapital(parsed);
                  if (typeof window !== 'undefined') {
                    window.localStorage.setItem(CAPITAL_STORAGE_KEY, String(parsed));
                  }
                } else {
                  setCapitalDraft(String(capital));
                }
              }}
              onKeyDown={(e) => {
                if (e.key === 'Enter') {
                  (e.currentTarget as HTMLInputElement).blur();
                }
              }}
            />
          </div>
          <p className={styles.cardSubtext}>
            Persisted to localStorage. Used as the default starting cash for backtests.
          </p>
        </article>

        <article className={styles.card}>
          <header className={styles.indexHeader}>
            <div>
              <h2 className={styles.cardLabel}>Index Data</h2>
              <p className={styles.cardSubtext}>
                22 NSE indices · auto-updated quarterly
              </p>
            </div>
            <Button
              variant="ghost"
              onClick={handleRefresh}
              disabled={isRefreshing}
            >
              {isRefreshing ? 'Refreshing…' : 'Refresh Now'}
            </Button>
          </header>

          <div className={styles.indexGrid} role="list">
            {indices.length === 0 ? (
              <p className={styles.indexEmpty}>No index data loaded.</p>
            ) : (
              indices.map((idx) => (
                <div key={idx.alias} className={styles.indexRow} role="listitem">
                  <span className={styles.indexName}>{idx.displayName}</span>
                  <span className={styles.indexMeta}>
                    {idx.symbolCount > 0 ? `${idx.symbolCount} symbols` : '–'}
                  </span>
                  <span className={styles.indexUpdated}>
                    {idx.lastUpdated === 'never'
                      ? 'never'
                      : formatDate(idx.lastUpdated)}
                  </span>
                </div>
              ))
            )}
          </div>

          {refreshMsg && (
            <p
              className={
                refreshMsgKind === 'error'
                  ? styles.indexMsgError
                  : styles.indexMsgOk
              }
              role="status"
            >
              {refreshMsg}
            </p>
          )}
        </article>

        <article className={`${styles.card} ${styles.aboutCard}`}>
          <h2 className={styles.aboutTitle}>AlgoMLN</h2>
          <p className={styles.version}>v0.1.0</p>
          <p className={styles.tagline}>Built for algo traders, by algo traders.</p>
        </article>
      </main>
    </div>
  );
}

function formatDate(iso: string): string {
  // Best-effort short date (YYYY-MM-DD or anything Date can parse).
  const d = new Date(iso);
  if (Number.isNaN(d.getTime())) return iso;
  const y = d.getFullYear();
  const m = String(d.getMonth() + 1).padStart(2, '0');
  const day = String(d.getDate()).padStart(2, '0');
  return `${y}-${m}-${day}`;
}
