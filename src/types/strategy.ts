// Every indicator the DSL supports
export type IndicatorKind =
  | 'sma'
  | 'ema'
  | 'rsi'
  | 'atr'
  | 'vwap'
  | 'bb_upper'
  | 'bb_lower'
  | 'bb_mid';

// Price fields usable as the right-hand side
export type PriceField = 'close' | 'open' | 'high' | 'low' | 'volume';

// Comparison operators
export type CompareOp = '<' | '=' | '>';

// What the right side of a condition can be
export type RhsMode = 'ltp' | 'value';

// +/- for the right-hand side modifier
export type RhsSign = '+' | '-';

// Entry action mode
export type ActionMode = 'quantity' | 'money';

// Sell mode (only used for Exit)
export type SellMode = 'quantity' | 'money' | 'all';

// One complete condition + action row
export interface BuilderRule {
  id: string;

  // Condition (left side)
  indicator: IndicatorKind;
  period: number;
  op: CompareOp;

  // Condition (right side)
  rhsMode: RhsMode;
  rhsSign: RhsSign;
  rhsValue: number;

  // Action
  actionVerb: 'buy' | 'sell';
  actionMode: ActionMode | SellMode;
  actionQuantity: number;
}

// The full strategy as the builder holds it
export interface BuilderStrategy {
  name: string;
  entry: BuilderRule;
  exit: BuilderRule;
}

// What the Strategies screen shows for a deployed strategy
export interface DeployedStrategy {
  id: string;
  name: string;
  description: string;
  totalPnl: number;
  totalTrades: number;
  modes: Array<'paper' | 'live'>;
  status: 'running' | 'paused';
  dslSource: string;
}

// UI display name map (NOT the DSL name)
export const INDICATOR_DISPLAY: Record<IndicatorKind, string> = {
  sma: 'SMA',
  ema: 'EMA',
  rsi: 'RSI',
  atr: 'ATR',
  vwap: 'VWAP',
  bb_upper: 'BB Upper',
  bb_lower: 'BB Lower',
  bb_mid: 'BB Mid',
};

// All indicator kinds in display order
export const INDICATOR_ORDER: IndicatorKind[] = [
  'sma',
  'ema',
  'rsi',
  'atr',
  'vwap',
  'bb_upper',
  'bb_lower',
  'bb_mid',
];
