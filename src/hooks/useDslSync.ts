import { useEffect, useMemo, useState } from 'react';
import { validateDsl, isTauri } from '../types/tauri';
import type {
  BuilderRule,
  BuilderStrategy,
  CompareOp,
  IndicatorKind,
} from '../types/strategy';

const INDICATOR_DSL: Record<IndicatorKind, string> = {
  sma: 'ma',
  ema: 'ema',
  rsi: 'rsi',
  atr: 'atr',
  vwap: 'vwap',
  bb_upper: 'bb_upper',
  bb_lower: 'bb_lower',
  bb_mid: 'bb_mid',
};

const OP_DSL: Record<CompareOp, string> = {
  '<': '<',
  '=': '==',
  '>': '>',
};

function ruleToConditionDsl(rule: BuilderRule): string {
  const indicator = `${INDICATOR_DSL[rule.indicator]}(${rule.period})`;
  const op = OP_DSL[rule.op];
  const rhs = rule.rhsMode === 'ltp' ? 'close' : String(rule.rhsValue);
  return `${indicator} ${op} ${rhs}`;
}

function ruleToActionDsl(rule: BuilderRule): string {
  if (rule.actionMode === 'all') {
    return 'SELL ALL';
  }
  const verb = rule.actionVerb === 'buy' ? 'BUY' : 'SELL';
  return `${verb} ${rule.actionQuantity}`;
}

export function strategyToDsl(strategy: BuilderStrategy): string {
  const entryCondition = ruleToConditionDsl(strategy.entry);
  const entryAction = ruleToActionDsl(strategy.entry);
  const exitCondition = ruleToConditionDsl(strategy.exit);
  const exitAction = ruleToActionDsl(strategy.exit);

  return [
    `# Entry`,
    `WHEN ${entryCondition}`,
    entryAction,
    ``,
    `# Exit`,
    `WHEN ${exitCondition}`,
    exitAction,
  ].join('\n');
}

export interface UseDslSyncResult {
  dsl: string;
  isValid: boolean;
  validationErrors: string[];
}

/**
 * Derives a live DSL string from builder state. Debounces validation
 * requests to the backend by 500ms so we don't spam IPC on every keystroke.
 *
 * When running outside Tauri (e.g. `npm run dev` without the Rust shell),
 * the synchronous fallback always reports valid because the builder only
 * emits grammars we can construct locally.
 */
export function useDslSync(strategy: BuilderStrategy): UseDslSyncResult {
  const dsl = useMemo(() => strategyToDsl(strategy), [strategy]);
  const [validationErrors, setValidationErrors] = useState<string[]>([]);

  useEffect(() => {
    if (!isTauri()) {
      setValidationErrors([]);
      return;
    }
    let cancelled = false;
    const handle = setTimeout(() => {
      validateDsl(dsl)
        .then((errs) => {
          if (!cancelled) setValidationErrors(errs);
        })
        .catch((err) => {
          if (!cancelled) {
            setValidationErrors([String(err)]);
          }
        });
    }, 500);
    return () => {
      cancelled = true;
      clearTimeout(handle);
    };
  }, [dsl]);

  return {
    dsl,
    isValid: validationErrors.length === 0,
    validationErrors,
  };
}

// ----- Simple DSL -> BuilderStrategy parser for round-trip support -----

const VALID_OPERATORS: ReadonlyArray<CompareOp> = ['<', '=', '>'];
const VALID_INDICATORS: ReadonlyArray<IndicatorKind> = [
  'sma',
  'ema',
  'rsi',
  'atr',
  'vwap',
  'bb_upper',
  'bb_lower',
  'bb_mid',
];

const DSL_TO_INDICATOR: Record<string, IndicatorKind> = {
  ma: 'sma',
  ema: 'ema',
  rsi: 'rsi',
  atr: 'atr',
  vwap: 'vwap',
  bb_upper: 'bb_upper',
  bb_lower: 'bb_lower',
  bb_mid: 'bb_mid',
};

const DSL_TO_OP: Record<string, CompareOp> = {
  '<': '<',
  '==': '=',
  '=': '=',
  '>': '>',
};

interface ParsedRule {
  indicator: IndicatorKind;
  period: number;
  op: CompareOp;
  rhsMode: 'ltp' | 'value';
  rhsValue: number;
  actionVerb: 'buy' | 'sell';
  actionMode: 'quantity' | 'money' | 'all';
  actionQuantity: number;
}

function uuid(): string {
  if (typeof crypto !== 'undefined' && 'randomUUID' in crypto) {
    return crypto.randomUUID();
  }
  return `id-${Date.now()}-${Math.random().toString(36).slice(2)}`;
}

function parseCondition(token: string): {
  indicator: IndicatorKind;
  period: number;
  op: CompareOp;
  rhsMode: 'ltp' | 'value';
  rhsValue: number;
} | null {
  const match = token.match(
    /^\s*([a-z_]+)\s*\(\s*(\d+)\s*\)\s*(<|==|=|>)\s*(.+?)\s*$/
  );
  if (!match) return null;
  const [, fn, periodStr, opStr, rhsStr] = match;
  const ind = DSL_TO_INDICATOR[fn];
  if (!ind || !VALID_INDICATORS.includes(ind)) return null;
  const op = DSL_TO_OP[opStr];
  if (!op || !VALID_OPERATORS.includes(op)) return null;
  const period = parseInt(periodStr, 10);
  if (!Number.isFinite(period)) return null;
  if (rhsStr === 'close') {
    return { indicator: ind, period, op, rhsMode: 'ltp', rhsValue: 0 };
  }
  const num = parseFloat(rhsStr);
  if (!Number.isFinite(num)) return null;
  return { indicator: ind, period, op, rhsMode: 'value', rhsValue: num };
}

function parseAction(token: string): {
  actionVerb: 'buy' | 'sell';
  actionMode: 'quantity' | 'money' | 'all';
  actionQuantity: number;
} | null {
  const trimmed = token.trim().toUpperCase();
  if (trimmed === 'SELL ALL') {
    return { actionVerb: 'sell', actionMode: 'all', actionQuantity: 0 };
  }
  const m = trimmed.match(/^(BUY|SELL)\s+(\d+)$/);
  if (!m) return null;
  const verb = m[1].toLowerCase() as 'buy' | 'sell';
  const qty = parseInt(m[2], 10);
  return { actionVerb: verb, actionMode: 'quantity', actionQuantity: qty };
}

/**
 * Best-effort parse of a `.algomln` source string back into a BuilderStrategy.
 * Returns `null` if the source is more complex than the visual builder can
 * represent (multiple rules, AND/OR, cross-above, etc.). In that case the
 * caller should keep the source available for the coder and mark the builder
 * as "advanced mode".
 */
export function parseDslToStrategy(source: string): BuilderStrategy | null {
  const lines = source
    .split('\n')
    .map((l) => l.trim())
    .filter((l) => l && !l.startsWith('#'));

  if (lines.length < 4) return null;

  // Strip the two 'WHEN' and two action lines
  const whenLines = lines.filter((l) => l.toUpperCase().startsWith('WHEN '));
  if (whenLines.length !== 2) return null;
  const actionLines = lines.filter(
    (l) => l.toUpperCase().startsWith('BUY ') || l.toUpperCase().startsWith('SELL ')
  );
  if (actionLines.length !== 2) return null;

  const entryCondition = parseCondition(whenLines[0].slice(5));
  const entryAction = parseAction(actionLines[0]);
  const exitCondition = parseCondition(whenLines[1].slice(5));
  const exitAction = parseAction(actionLines[1]);
  if (!entryCondition || !entryAction || !exitCondition || !exitAction) return null;
  if (entryAction.actionVerb !== 'buy') return null;
  if (exitAction.actionVerb !== 'sell') return null;

  return {
    name: 'Loaded Strategy',
    entry: {
      id: uuid(),
      indicator: entryCondition.indicator,
      period: entryCondition.period,
      op: entryCondition.op,
      rhsMode: entryCondition.rhsMode,
      rhsSign: '+',
      rhsValue: entryCondition.rhsMode === 'value' ? entryCondition.rhsValue : 0,
      actionVerb: 'buy',
      actionMode: entryAction.actionMode === 'all' ? 'quantity' : entryAction.actionMode,
      actionQuantity:
        entryAction.actionMode === 'quantity' ? entryAction.actionQuantity : 0,
    },
    exit: {
      id: uuid(),
      indicator: exitCondition.indicator,
      period: exitCondition.period,
      op: exitCondition.op,
      rhsMode: exitCondition.rhsMode,
      rhsSign: '+',
      rhsValue: exitCondition.rhsMode === 'value' ? exitCondition.rhsValue : 0,
      actionVerb: 'sell',
      actionMode: exitAction.actionMode,
      actionQuantity: exitAction.actionQuantity,
    },
  };
}
