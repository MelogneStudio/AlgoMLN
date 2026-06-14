import type { CSSProperties, ReactNode } from 'react';
import styles from './AppWindow.module.css';

interface AppWindowProps {
  scale: number;
  children: ReactNode;
}

export function AppWindow({ scale, children }: AppWindowProps) {
  const style = { '--ui-scale': String(scale) } as CSSProperties;
  return (
    <div className={styles.window} style={style}>
      {children}
    </div>
  );
}
