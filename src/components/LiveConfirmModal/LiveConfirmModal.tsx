import { useEffect, useRef, useState } from 'react';
import { Button } from '../../components/Button/Button';
import { requestLiveStart, confirmLiveStart, acknowledgeLiveTrading, isTauri } from '../../types/tauri';
import styles from './LiveConfirmModal.module.css';

interface LiveConfirmModalProps {
  strategyId: string;
  strategyName: string;
  onSuccess: () => void;
  onCancel: () => void;
}

type ModalStep = 'preflight' | 'ack' | 'confirm' | 'error';

export function LiveConfirmModal({
  strategyId,
  strategyName,
  onSuccess,
  onCancel,
}: LiveConfirmModalProps) {
  const [step, setStep] = useState<ModalStep>('preflight');
  const [token, setToken] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [countdown, setCountdown] = useState(3);
  const countdownRef = useRef<number | null>(null);
  const browserMode = !isTauri();

  useEffect(() => {
    if (step === 'confirm' && countdown > 0) {
      countdownRef.current = window.setInterval(() => {
        setCountdown((c) => {
          if (c <= 1) {
            if (countdownRef.current !== null) {
              window.clearInterval(countdownRef.current);
              countdownRef.current = null;
            }
            return 0;
          }
          return c - 1;
        });
      }, 1000);
    }
    return () => {
      if (countdownRef.current !== null) {
        window.clearInterval(countdownRef.current);
        countdownRef.current = null;
      }
    };
  }, [step, countdown]);

  const clearError = () => setError(null);

  const handleRunChecks = async () => {
    setError(null);
    try {
      const res = await requestLiveStart(strategyId);
      setToken(res.token);
      if (res.requiresAck) {
        setStep('ack');
      } else {
        setCountdown(3);
        setStep('confirm');
      }
    } catch (e) {
      setError(String(e));
    }
  };

  const handleAckProceed = async () => {
    setError(null);
    try {
      await acknowledgeLiveTrading();
      setCountdown(3);
      setStep('confirm');
    } catch (e) {
      setError(String(e));
      setStep('preflight');
    }
  };

  const handleConfirm = async () => {
    if (token === null) {
      setError('No token available');
      setStep('preflight');
      return;
    }
    setError(null);
    try {
      await confirmLiveStart(strategyId, token);
      onSuccess();
    } catch (e) {
      setError(String(e));
      setToken(null);
      setStep('preflight');
    }
  };

  const handleCancel = () => {
    if (countdownRef.current !== null) {
      window.clearInterval(countdownRef.current);
      countdownRef.current = null;
    }
    onCancel();
  };

  if (browserMode) {
    return (
      <div className={styles.overlay} onClick={handleCancel}>
        <div className={styles.modal} onClick={(e) => e.stopPropagation()}>
          <header className={styles.header}>
            <h2 className={styles.title}>Start Live Trading</h2>
          </header>
          <div className={styles.body}>
            <div className={styles.browserNotice}>
              <svg viewBox="0 0 24 24" width="32" height="32" fill="none" className={styles.noticeIcon}>
                <circle cx="12" cy="12" r="10" stroke="currentColor" strokeWidth="1.5" />
                <path d="M12 8v4M12 16h.01" stroke="currentColor" strokeWidth="1.5" strokeLinecap="round" />
              </svg>
              <p>Live trading is only available in the desktop app.</p>
              <p className={styles.noticeHint}>Run <code>npm run tauri dev</code> to open the Tauri window.</p>
            </div>
          </div>
          <footer className={styles.footer}>
            <Button variant="ghost" onClick={handleCancel}>Cancel</Button>
          </footer>
        </div>
      </div>
    );
  }

  const renderPreflight = () => (
    <>
      <header className={styles.header}>
        <h2 className={styles.title}>Start Live Trading</h2>
      </header>
      <div className={styles.body}>
        <p className={styles.subtitle}>Pre-flight checks for <strong>{strategyName}</strong></p>
        <ul className={styles.checklist}>
          <li>Market hours check</li>
          <li>Broker connectivity check</li>
          <li>Symbol map check</li>
          <li>Segment check (NSE equity only)</li>
          <li>Risk controls check</li>
          <li>MAX_DAILY_LOSS declared</li>
          <li>Broker loss-tracking healthy</li>
        </ul>
        <div className={styles.warningBox}>
          <svg viewBox="0 0 24 24" width="18" height="18" fill="none" className={styles.warningIcon}>
            <path d="M12 2v20M12 8v8M12 16h.01" stroke="currentColor" strokeWidth="1.5" strokeLinecap="round" />
          </svg>
          <div className={styles.warningText}>
            <strong>⚠ Live trading places real orders with real money.</strong> Losses may exceed your configured limits in fast markets.
            Stopping the session does <strong>not</strong> auto-close positions or cancel pending orders — you must handle those in your broker app.
            Make sure you have reviewed your strategy's backtest.
          </div>
          {/* L4 (audit): sessions that start in the last minute before close
              may still place an order on the 15:30 candle boundary — NSE
              rejects post-close orders, so the broker app will show the
              rejection rather than the engine silently dropping it. */}
          <div className={styles.warningText}>
            <strong>Market-hours boundary.</strong> Live trading is allowed
            09:15–15:30 IST on NSE trading days. Sessions started within the
            last minute before close may still attempt to place an order on
            the 15:30 candle, which the exchange will reject.
          </div>
        </div>
        {error && <div className={styles.error}>{error}</div>}
      </div>
      <footer className={styles.footer}>
        <Button variant="ghost" onClick={handleCancel}>Cancel</Button>
        <Button variant="primary" onClick={handleRunChecks} disabled={step !== 'preflight'}>Run Checks</Button>
      </footer>
    </>
  );

  const renderAck = () => (
    <>
      <header className={styles.header}>
        <h2 className={styles.title}>First Live Trade Warning</h2>
      </header>
      <div className={styles.body}>
        <p className={styles.ackBody}>
          You are about to place your first live order on AlgoMLN. Once confirmed, real orders will be sent to Dhan on your behalf.
          Paper trading is always available and recommended for new strategies. Do you understand and accept the risks?
        </p>
        {error && <div className={styles.error}>{error}</div>}
      </div>
      <footer className={styles.footer}>
        <Button variant="ghost" onClick={handleCancel}>Cancel</Button>
        <Button variant="primary" onClick={handleAckProceed}>I Understand — Proceed</Button>
      </footer>
    </>
  );

  const renderConfirm = () => (
    <>
      <header className={styles.header}>
        <h2 className={styles.title}>Confirm Live Start</h2>
      </header>
      <div className={styles.body}>
        <div className={styles.confirmDetails}>
          <div className={styles.detailRow}>
            <span className={styles.detailLabel}>Strategy</span>
            <span className={styles.detailValue}>{strategyName}</span>
          </div>
          <div className={styles.detailRow}>
            <span className={styles.detailLabel}>Mode</span>
            <span className={styles.detailValue} style={{ color: '#c47a4a' }}>LIVE</span>
          </div>
        </div>
        <p className={styles.reminder}>
          Reminder: stopping the session does <strong>NOT</strong> auto-close open positions or cancel pending orders.
        </p>
        {error && <div className={styles.error}>{error}</div>}
      </div>
      <footer className={styles.footer}>
        <Button variant="ghost" onClick={handleCancel}>Cancel</Button>
        <Button
          variant="primary"
          onClick={handleConfirm}
          disabled={countdown > 0}
          className={countdown > 0 ? styles.btnCountdown : ''}
        >
          {countdown > 0 ? `Wait (${countdown})…` : 'Confirm — Go Live'}
        </Button>
      </footer>
    </>
  );

  return (
    <div className={styles.overlay} onClick={handleCancel}>
      <div className={styles.modal} onClick={(e) => e.stopPropagation()}>
        {step === 'preflight' && renderPreflight()}
        {step === 'ack' && renderAck()}
        {step === 'confirm' && renderConfirm()}
      </div>
    </div>
  );
}