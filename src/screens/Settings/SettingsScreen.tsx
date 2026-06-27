import { useState } from 'react';
import {
  CAPITAL_STORAGE_KEY,
  loadSavedCapital,
  saveCapital,
} from '../../lib/scaling';
import styles from './SettingsScreen.module.css';

export function SettingsScreen() {
  const [capital, setCapital] = useState<number>(() => loadSavedCapital());
  const [capitalDraft, setCapitalDraft] = useState<string>(String(capital));

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

        <article className={`${styles.card} ${styles.aboutCard}`}>
          <h2 className={styles.aboutTitle}>AlgoMLN</h2>
          <p className={styles.version}>v0.1.0</p>
          <p className={styles.tagline}>Built for algo traders, by algo traders.</p>
        </article>
      </main>
    </div>
  );
}
