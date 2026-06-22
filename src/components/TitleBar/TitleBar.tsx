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
  return (
    <header className={styles.bar}>
      <div className={styles.left}>
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
    </header>
  );
}
