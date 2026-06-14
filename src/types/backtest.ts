export interface BacktestSummary {
  initialCash: number;
  finalCash: number;
  totalReturnPct: number;
  totalTrades: number;
  buyCount: number;
  sellCount: number;
  closedTrades: number;
  winningTrades: number;
  losingTrades: number;
  breakevenTrades: number;
  winRatePct: number;
  totalRealizedPnl: number;
  grossProfit: number;
  grossLoss: number;
  profitFactor: number;
  avgWin: number;
  avgLoss: number;
  largestWin: number;
  largestLoss: number;
  expectancy: number;
  maxDrawdown: number;
  maxDrawdownPct: number;
  maxConsecutiveWins: number;
  maxConsecutiveLosses: number;
  totalCandlesProcessed: number;
  candlesPerTrade: number;
  skippedNoPosition: number;
}

export interface PaperTrade {
  id: string;
  timestamp: string;
  symbol: string;
  side: 'buy' | 'sell';
  quantity: number;
  price: number;
  pnl: number | null;
}

export interface BacktestResult {
  tradeHistory: PaperTrade[];
  finalCash: number;
  initialCash: number;
  totalRealizedPnl: number;
  totalCandlesProcessed: number;
  summary: BacktestSummary;
}
