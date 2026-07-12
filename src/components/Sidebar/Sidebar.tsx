import { SIDEBAR_FORCE_COLLAPSE_THRESHOLD } from '../../lib/scaling';
import type { Screen } from '../../App';
import styles from './Sidebar.module.css';

interface SidebarProps {
  collapsed: boolean;
  forcedCollapsed: boolean;
  scale: number;
  active: Screen;
  onNavigate: (screen: Screen) => void;
}

interface NavItem {
  id: Screen;
  label: string;
  icon: React.ReactNode;
}

const ITEMS: NavItem[] = [
  {
    id: 'builder',
    label: 'Builder',
    icon: (
      <svg viewBox="0 0 24 24" width="22" height="22" fill="none">
        <path
          d="M3 17l6-6 4 4 8-8M14 7h7v7"
          stroke="currentColor"
          strokeWidth="1.8"
          strokeLinecap="round"
          strokeLinejoin="round"
        />
      </svg>
    ),
  },
  {
    id: 'strategies',
    label: 'Strategies',
    icon: (
      <svg viewBox="0 0 24 24" width="22" height="22" fill="none">
        <rect
          x="3"
          y="4"
          width="18"
          height="5"
          rx="1.5"
          stroke="currentColor"
          strokeWidth="1.8"
        />
        <rect
          x="3"
          y="11"
          width="18"
          height="5"
          rx="1.5"
          stroke="currentColor"
          strokeWidth="1.8"
        />
        <rect
          x="3"
          y="18"
          width="18"
          height="3"
          rx="1.2"
          stroke="currentColor"
          strokeWidth="1.8"
        />
      </svg>
    ),
  },
  {
    id: 'plugins',
    label: 'Plugins',
    icon: (
      <svg viewBox="0 0 24 24" width="22" height="22" fill="none">
        <path
          d="M10 3v4M14 3v4M6 7h12v5a6 6 0 0 1-12 0V7zM12 18v3"
          stroke="currentColor"
          strokeWidth="1.8"
          strokeLinecap="round"
          strokeLinejoin="round"
        />
      </svg>
    ),
  },
  {
    id: 'settings',
    label: 'Settings',
    icon: (
      <svg viewBox="0 0 24 24" width="22" height="22" fill="none">
        <circle cx="12" cy="12" r="3" stroke="currentColor" strokeWidth="1.8" />
        <path
          d="M19.4 15a1.7 1.7 0 0 0 .3 1.8l.1.1a2 2 0 1 1-2.8 2.8l-.1-.1a1.7 1.7 0 0 0-1.8-.3 1.7 1.7 0 0 0-1 1.5V21a2 2 0 0 1-4 0v-.1a1.7 1.7 0 0 0-1.1-1.5 1.7 1.7 0 0 0-1.8.3l-.1.1a2 2 0 1 1-2.8-2.8l.1-.1a1.7 1.7 0 0 0 .3-1.8 1.7 1.7 0 0 0-1.5-1H3a2 2 0 0 1 0-4h.1a1.7 1.7 0 0 0 1.5-1.1 1.7 1.7 0 0 0-.3-1.8l-.1-.1a2 2 0 1 1 2.8-2.8l.1.1a1.7 1.7 0 0 0 1.8.3H9a1.7 1.7 0 0 0 1-1.5V3a2 2 0 0 1 4 0v.1a1.7 1.7 0 0 0 1 1.5 1.7 1.7 0 0 0 1.8-.3l.1-.1a2 2 0 1 1 2.8 2.8l-.1.1a1.7 1.7 0 0 0-.3 1.8V9a1.7 1.7 0 0 0 1.5 1H21a2 2 0 0 1 0 4h-.1a1.7 1.7 0 0 0-1.5 1z"
          stroke="currentColor"
          strokeWidth="1.6"
        />
      </svg>
    ),
  },
];

export function Sidebar({
  collapsed,
  forcedCollapsed,
  scale,
  active,
  onNavigate,
}: SidebarProps) {
  const isCollapsed = forcedCollapsed || collapsed;
  const logoWidth = isCollapsed ? 56 : 204;
  const logoText = isCollapsed ? 'λ' : 'λlgoMLN';

  // Scale guardrail: scale-aware layout. The spec hard-locks the sidebar
  // width and hides the toggle at scale < 0.75.
  const forceCollapseByScale = scale < SIDEBAR_FORCE_COLLAPSE_THRESHOLD;
  const effectiveCollapsed = forcedCollapsed || forceCollapseByScale || collapsed;

  return (
    <aside
      className={`${styles.sidebar} ${effectiveCollapsed ? styles.collapsed : styles.expanded}`}
      aria-label="Primary navigation"
    >
      <div className={styles.inner}>
        <div
          className={styles.logo}
          style={{ width: `${logoWidth}px` }}
          aria-label="AlgoMLN"
        >
          <span className={styles.logoMark}>{logoText}</span>
        </div>

        <nav className={styles.nav}>
          {ITEMS.map((item) => {
            const isActive = item.id === active;
            return (
              <button
                type="button"
                key={item.id}
                className={`${styles.item} ${isActive ? styles.itemActive : ''}`}
                onClick={() => onNavigate(item.id)}
                aria-current={isActive ? 'page' : undefined}
                aria-label={item.label}
              >
                <span className={styles.itemIcon}>{item.icon}</span>
                {!isCollapsed && (
                  <span className={styles.itemLabel}>{item.label}</span>
                )}
              </button>
            );
          })}
        </nav>

        <div className={styles.separator} aria-hidden />

        <div
          className={styles.avatar}
          aria-label="Profile"
          role="img"
        >
          <span className={styles.avatarInitial}>A</span>
        </div>
      </div>
    </aside>
  );
}
