import { getCurrentWindow } from '@tauri-apps/api/window';
import styles from './TitleBar.module.css';

interface TitleBarProps {
  sidebarCollapsed: boolean;
  onToggleSidebar: () => void;
  canToggle: boolean;
}

export function TitleBar({
  sidebarCollapsed,
  onToggleSidebar,
  canToggle,
}: TitleBarProps) {
  const handleMinimize = async () => {
    try {
      await getCurrentWindow().minimize();
    } catch (err) {
      console.warn('minimize failed:', err);
    }
  };

  const handleClose = async () => {
    try {
      await getCurrentWindow().close();
    } catch (err) {
      console.warn('close failed:', err);
    }
  };

  const handleScreenshot = async () => {
    // Best-effort: Tauri v2 doesn't expose a simple capture helper on all
    // platforms, so this is intentionally a no-op if unsupported.
    console.info('Screenshot requested (not supported in this build)');
  };

  return (
    <header className={styles.bar} data-tauri-drag-region>
      <div className={styles.left} data-tauri-drag-region>
        {canToggle && (
          <button
            type="button"
            className={styles.toggle}
            onClick={onToggleSidebar}
            aria-label={sidebarCollapsed ? 'Expand sidebar' : 'Collapse sidebar'}
          >
            <svg viewBox="0 0 24 24" width="20" height="20" fill="none">
              <path
                d="M4 7h16M4 12h16M4 17h16"
                stroke="currentColor"
                strokeWidth="2"
                strokeLinecap="round"
              />
            </svg>
          </button>
        )}
      </div>

      <div className={styles.dragFill} data-tauri-drag-region />

      <div className={styles.right} data-tauri-drag-region={false}>
        <div className={styles.controls}>
          <button
            type="button"
            className={styles.controlBtn}
            onClick={handleMinimize}
            aria-label="Minimize"
          >
            <svg viewBox="0 0 16 16" width="14" height="14" fill="none">
              <path
                d="M3 8h10"
                stroke="currentColor"
                strokeWidth="1.5"
                strokeLinecap="round"
              />
            </svg>
          </button>
          <button
            type="button"
            className={styles.controlBtn}
            onClick={handleScreenshot}
            aria-label="Screenshot"
          >
            <svg viewBox="0 0 16 16" width="14" height="14" fill="none">
              <rect
                x="2"
                y="4"
                width="12"
                height="9"
                rx="1.5"
                stroke="currentColor"
                strokeWidth="1.3"
              />
              <circle cx="8" cy="8.5" r="2" stroke="currentColor" strokeWidth="1.3" />
            </svg>
          </button>
          <button
            type="button"
            className={`${styles.controlBtn} ${styles.close}`}
            onClick={handleClose}
            aria-label="Close"
          >
            <svg viewBox="0 0 16 16" width="14" height="14" fill="none">
              <path
                d="M4 4l8 8M12 4l-8 8"
                stroke="currentColor"
                strokeWidth="1.5"
                strokeLinecap="round"
              />
            </svg>
          </button>
        </div>
      </div>
    </header>
  );
}
