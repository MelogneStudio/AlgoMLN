import { useCallback, useState } from 'react';
import type { BuilderRule, BuilderStrategy } from '../types/strategy';
import { parseDslToStrategy } from './useDslSync';

function uuid(): string {
  if (typeof crypto !== 'undefined' && 'randomUUID' in crypto) {
    return crypto.randomUUID();
  }
  return `id-${Date.now()}-${Math.random().toString(36).slice(2)}`;
}

function makeRule(overrides: Partial<BuilderRule>): BuilderRule {
  return {
    id: uuid(),
    indicator: 'sma',
    period: 14,
    op: '<',
    rhsMode: 'ltp',
    rhsSign: '+',
    rhsValue: 70,
    actionVerb: 'buy',
    actionMode: 'quantity',
    actionQuantity: 20,
    ...overrides,
  };
}

const DEFAULT_ENTRY_RULE: BuilderRule = makeRule({
  indicator: 'sma',
  period: 14,
  op: '<',
  rhsMode: 'ltp',
  rhsSign: '+',
  rhsValue: 70,
  actionVerb: 'buy',
  actionMode: 'quantity',
  actionQuantity: 20,
});

const DEFAULT_EXIT_RULE: BuilderRule = makeRule({
  indicator: 'sma',
  period: 14,
  op: '<',
  rhsMode: 'ltp',
  rhsSign: '+',
  rhsValue: 70,
  actionVerb: 'sell',
  actionMode: 'quantity',
  actionQuantity: 20,
});

const DEFAULT_STRATEGY: BuilderStrategy = {
  name: 'Untitled Strategy',
  entry: DEFAULT_ENTRY_RULE,
  exit: DEFAULT_EXIT_RULE,
};

export interface UseStrategyBuilderResult {
  strategy: BuilderStrategy;
  setStrategyName: (name: string) => void;
  setEntryRule: (patch: Partial<BuilderRule>) => void;
  setExitRule: (patch: Partial<BuilderRule>) => void;
  resetStrategy: () => void;
  loadFromDsl: (dsl: string) => boolean;
  isAdvancedMode: boolean;
}

export function useStrategyBuilder(): UseStrategyBuilderResult {
  const [strategy, setStrategy] = useState<BuilderStrategy>(DEFAULT_STRATEGY);
  const [isAdvancedMode, setAdvancedMode] = useState(false);

  const setStrategyName = useCallback((name: string) => {
    setStrategy((prev) => ({ ...prev, name }));
  }, []);

  const setEntryRule = useCallback((patch: Partial<BuilderRule>) => {
    setStrategy((prev) => ({
      ...prev,
      entry: { ...prev.entry, ...patch },
    }));
  }, []);

  const setExitRule = useCallback((patch: Partial<BuilderRule>) => {
    setStrategy((prev) => ({
      ...prev,
      exit: { ...prev.exit, ...patch },
    }));
  }, []);

  const resetStrategy = useCallback(() => {
    setStrategy({
      name: 'Untitled Strategy',
      entry: makeRule({
        indicator: 'sma',
        period: 14,
        op: '<',
        rhsMode: 'ltp',
        rhsSign: '+',
        rhsValue: 70,
        actionVerb: 'buy',
        actionMode: 'quantity',
        actionQuantity: 20,
      }),
      exit: makeRule({
        indicator: 'sma',
        period: 14,
        op: '<',
        rhsMode: 'ltp',
        rhsSign: '+',
        rhsValue: 70,
        actionVerb: 'sell',
        actionMode: 'quantity',
        actionQuantity: 20,
      }),
    });
    setAdvancedMode(false);
  }, []);

  const loadFromDsl = useCallback((dsl: string): boolean => {
    const parsed = parseDslToStrategy(dsl);
    if (!parsed) {
      // Could not parse cleanly — keep DSL accessible via advanced mode
      setAdvancedMode(true);
      return false;
    }
    setStrategy(parsed);
    setAdvancedMode(false);
    return true;
  }, []);

  return {
    strategy,
    setStrategyName,
    setEntryRule,
    setExitRule,
    resetStrategy,
    loadFromDsl,
    isAdvancedMode,
  };
}
