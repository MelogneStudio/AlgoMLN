import { useEffect, useState } from 'react';
import {
  listPlugins,
  enablePlugin,
  disablePlugin,
  reloadPlugins,
  isTauri,
} from '../../types/tauri';
import type {
  PluginListEntry,
  PluginStatus,
} from '../../types/plugin';
import styles from './PluginsScreen.module.css';

const DEMO_PLUGINS: PluginListEntry[] = [
  { meta: { id: 'example-plugin', name: 'Example Plugin', version: {major:0,minor:1,patch:0}, description: 'A demo indicator plugin', author: 'AlgoMLN' }, status: 'Enabled', capabilities: ['Indicators','Storage'] },
  { meta: { id: 'rsi-divergence', name: 'RSI Divergence', version: {major:1,minor:0,patch:0}, description: 'Detects RSI divergence signals', author: 'Community' }, status: 'Disabled', capabilities: ['Indicators','Events'] },
  { meta: { id: 'telegram-alerts', name: 'Telegram Alerts', version: {major:0,minor:3,patch:1}, description: 'Sends trade alerts to Telegram', author: 'Community' }, status: { Failed: 'Missing TELEGRAM_TOKEN in plugin config' }, capabilities: ['Events','Scheduler'] },
];

function getStatusLabel(status: PluginStatus): string {
  if (typeof status === 'object') return 'Failed';
  return status;
}

function getStatusClass(status: PluginStatus): string {
  if (typeof status === 'object') return 'statusFailed';
  switch (status) {
    case 'Enabled':
      return 'statusEnabled';
    case 'Disabled':
      return 'statusDisabled';
    case 'Loaded':
      return 'statusLoaded';
  }
}

function isEnabled(status: PluginStatus): boolean {
  return status === 'Enabled';
}

function isFailed(status: PluginStatus): boolean {
  return typeof status === 'object' && 'Failed' in status;
}

function getFailureMsg(status: PluginStatus): string | null {
  return typeof status === 'object' ? status.Failed : null;
}

export function PluginsScreen() {
  const [plugins, setPlugins] = useState<PluginListEntry[]>([]);
  const [loading, setLoading] = useState(true);
  const [reloading, setReloading] = useState(false);
  const [togglingId, setTogglingId] = useState<string | null>(null);
  const [reloadErrors, setReloadErrors] = useState<string[]>([]);

  useEffect(() => {
    let cancelled = false;
    async function load() {
      if (isTauri()) {
        try {
          const list = await listPlugins();
          if (!cancelled) setPlugins(list);
        } catch (err) {
          console.error('Failed to list plugins:', err);
        }
      } else {
        if (!cancelled) setPlugins(DEMO_PLUGINS);
      }
      if (!cancelled) setLoading(false);
    }
    void load();
    return () => {
      cancelled = true;
    };
  }, []);

  async function handleToggle(entry: PluginListEntry) {
    setTogglingId(entry.meta.id);
    try {
      if (isEnabled(entry.status)) {
        await disablePlugin(entry.meta.id);
      } else {
        await enablePlugin(entry.meta.id);
      }
      const list = await listPlugins();
      setPlugins(list);
    } catch (err) {
      console.error('Failed to toggle plugin:', err);
    } finally {
      setTogglingId(null);
    }
  }

  async function handleReload() {
    setReloading(true);
    try {
      const errs = await reloadPlugins();
      const list = await listPlugins();
      setPlugins(list);
      setReloadErrors(errs);
    } catch (err) {
      console.error('Failed to reload plugins:', err);
    } finally {
      setReloading(false);
    }
  }

  return (
    <div className={styles.screen}>
      <div className={styles.header}>
        <h2 className={styles.title}>Plugins</h2>
        <button onClick={handleReload} disabled={reloading} className={styles.reloadBtn}>
          {reloading ? 'Reloading...' : 'Reload'}
        </button>
      </div>
      {reloadErrors.length > 0 && (
        <div className={styles.errorList}>
          {reloadErrors.map(e => <p key={e} className={styles.errorItem}>{e}</p>)}
        </div>
      )}
      <p className={styles.pluginsPath}>Plugins folder: %APPDATA%\com.algomln.app\plugins\</p>
      {loading ? <p className={styles.loadingText}>Loading plugins...</p> : (
        plugins.length === 0
          ? <p className={styles.emptyText}>No plugins installed.</p>
          : <div className={styles.list}>
              {plugins.map(entry => (
                <div key={entry.meta.id} className={styles.card}>
                  <div className={styles.cardHeader}>
                    <div>
                      <span className={styles.pluginName}>{entry.meta.name}</span>
                      <span className={styles.pluginId}>{entry.meta.id}</span>
                    </div>
                    <span className={`${styles.statusBadge} ${styles[getStatusClass(entry.status)]}`}>
                      {getStatusLabel(entry.status)}
                    </span>
                  </div>
                  <p className={styles.pluginDescription}>{entry.meta.description}</p>
                  <div className={styles.cardMeta}>
                    <span className={styles.metaItem}>v{entry.meta.version.major}.{entry.meta.version.minor}.{entry.meta.version.patch}</span>
                    <span className={styles.metaItem}>{entry.meta.author}</span>
                  </div>
                  <div className={styles.chips}>
                    {entry.capabilities.map(cap => (
                      <span key={cap} className={styles.chip}>{cap}</span>
                    ))}
                  </div>
                  {isFailed(entry.status) && (
                    <p className={styles.failureMsg}>{getFailureMsg(entry.status)}</p>
                  )}
                  <button
                    className={isEnabled(entry.status) ? styles.disableBtn : styles.enableBtn}
                    disabled={isFailed(entry.status) || togglingId === entry.meta.id}
                    onClick={() => handleToggle(entry)}
                  >
                    {togglingId === entry.meta.id ? '...' : isEnabled(entry.status) ? 'Disable' : 'Enable'}
                  </button>
                </div>
              ))}
            </div>
      )}
    </div>
  );
}
