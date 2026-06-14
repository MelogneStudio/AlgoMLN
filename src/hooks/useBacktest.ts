import { useCallback, useState } from 'react';
import { runBacktest, isTauri } from '../types/tauri';
import type { BacktestResult } from '../types/backtest';

export interface UseBacktestResult {
  run: (dslSource: string, symbol: string, initialCash: number) => Promise<void>;
  result: BacktestResult | null;
  isLoading: boolean;
  error: string | null;
  clear: () => void;
}

export function useBacktest(): UseBacktestResult {
  const [result, setResult] = useState<BacktestResult | null>(null);
  const [isLoading, setIsLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const run = useCallback(
    async (dslSource: string, symbol: string, initialCash: number) => {
      setIsLoading(true);
      setError(null);
      try {
        if (!isTauri()) {
          // No Tauri runtime (browser dev): synthesise a benign placeholder so
          // the UI is still demoable.
          setResult({
            tradeHistory: [],
            finalCash: initialCash,
            initialCash,
            totalRealizedPnl: 0,
            totalCandlesProcessed: 0,
            summary: emptySummary(initialCash),
          });
        } else {
          const res = await runBacktest(dslSource, symbol, initialCash);
          setResult(res);
        }
      } catch (err) {
        setError(err instanceof Error ? err.message : String(err));
        setResult(null);
      } finally {
        setIsLoading(false);
      }
    },
    []
  );

  const clear = useCallback(() => {
    setResult(null);
    setError(null);
  }, []);

  return { run, result, isLoading, error, clear };
}

function emptySummary(initialCash: number) {
  return {
    initialCash,
    finalCash: initialCash,
    totalReturnPct: 0,
    totalTrades: 0,
    buyCount: 0,
    sellCount: 0,
    closedTrades: 0,
    winningTrades: 0,
    losingTrades: 0,
    breakevenTrades: 0,
    winRatePct: 0,
    totalRealizedPnl: 0,
    grossProfit: 0,
    grossLoss: 0,
    profitFactor: 0,
    avgWin: 0,
    avgLoss: 0,
    largestWin: 0,
    largestLoss: 0,
    expectancy: 0,
    maxDrawdown: 0,
    maxDrawdownPct: 0,
    maxConsecutiveWins: 0,
    maxConsecutiveLosses: 0,
    totalCandlesProcessed: 0,
    candlesPerTrade: 0,
    skippedNoPosition: 0,
  };
}
