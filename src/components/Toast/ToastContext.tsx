import { createContext, useContext, useState, useCallback, ReactNode } from 'react';
import styles from './Toast.module.css';

export type ToastKind = 'info' | 'success' | 'warning' | 'error';

export interface Toast {
  id: number;
  kind: ToastKind;
  title: string;
  body: string;
  sticky: boolean;
}

interface ToastContextValue {
  toasts: Toast[];
  showToast: (toast: Omit<Toast, 'id'>) => void;
  dismissToast: (id: number) => void;
}

const ToastContext = createContext<ToastContextValue | null>(null);

export function ToastProvider({ children }: { children: ReactNode }) {
  const [toasts, setToasts] = useState<Toast[]>([]);
  const [nextId, setNextId] = useState(0);

  const showToast = useCallback(({ kind, title, body, sticky }: Omit<Toast, 'id'>) => {
    const id = nextId;
    setNextId((n) => n + 1);
    setToasts((t) => [...t, { id, kind, title, body, sticky }]);
    if (!sticky) {
      setTimeout(() => {
        setToasts((t) => t.filter((x) => x.id !== id));
      }, 5000);
    }
  }, [nextId]);

  const dismissToast = useCallback((id: number) => {
    setToasts((t) => t.filter((x) => x.id !== id));
  }, []);

  return (
    <ToastContext.Provider value={{ toasts, showToast, dismissToast }}>
      {children}
      <ToastList toasts={toasts} onDismiss={dismissToast} />
    </ToastContext.Provider>
  );
}

export function useToast() {
  const ctx = useContext(ToastContext);
  if (!ctx) throw new Error('useToast must be used within ToastProvider');
  return ctx;
}

function ToastList({ toasts, onDismiss }: { toasts: Toast[]; onDismiss: (id: number) => void }) {
  return (
    <div className={styles.toastContainer} aria-live="polite" aria-atomic="true">
      {toasts.map((t) => (
        <ToastCard key={t.id} toast={t} onDismiss={onDismiss} />
      ))}
    </div>
  );
}

function ToastCard({ toast, onDismiss }: { toast: Toast; onDismiss: (id: number) => void }) {
  const kindStyles: Record<ToastKind, string> = {
    info: styles.toastInfo,
    success: styles.toastSuccess,
    warning: styles.toastWarning,
    error: styles.toastError,
  };

  return (
    <div className={`${styles.toast} ${kindStyles[toast.kind]}`} role="alert">
      <div className={styles.toastContent}>
        <div className={styles.toastTitle}>{toast.title}</div>
        <div className={styles.toastBody}>{toast.body}</div>
      </div>
      {!toast.sticky && (
        <button
          type="button"
          className={styles.toastClose}
          onClick={() => onDismiss(toast.id)}
          aria-label="Dismiss"
        >
          <svg viewBox="0 0 16 16" width="14" height="14" fill="none">
            <path d="M4 4l8 8M12 4l-8 8" stroke="currentColor" strokeWidth="1.5" strokeLinecap="round" />
          </svg>
        </button>
      )}
    </div>
  );
}