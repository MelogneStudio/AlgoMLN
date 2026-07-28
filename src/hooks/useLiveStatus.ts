import { useCallback, useEffect, useRef, useState } from 'react';
import { getLiveStatus } from '../types/tauri';
import type { LiveStatusWire } from '../types/live';

export function useLiveStatus(pollIntervalMs = 5000) {
  const [status, setStatus] = useState<LiveStatusWire | null>(null);
  const [error, setError] = useState<string | null>(null);
  const timerRef = useRef<number | null>(null);

  const fetchOnce = useCallback(async () => {
    try {
      const s = await getLiveStatus();
      setStatus(s);
      setError(null);
    } catch (e) {
      setError(String(e));
    }
  }, []);

  useEffect(() => {
    fetchOnce();
    timerRef.current = window.setInterval(fetchOnce, pollIntervalMs);
    return () => {
      if (timerRef.current !== null) {
        window.clearInterval(timerRef.current);
        timerRef.current = null;
      }
    };
  }, [fetchOnce, pollIntervalMs]);

  return { status, error, refresh: fetchOnce };
}