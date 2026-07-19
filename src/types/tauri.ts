import { invoke } from '@tauri-apps/api/core';
import type { BacktestResult } from './backtest';
import type { DeployedStrategy } from './strategy';
import type { PluginListEntry } from './plugin';

// Run a backtest by passing raw DSL text
export async function runBacktest(
  dslSource: string,
  symbol: string,
  initialCash: number
): Promise<BacktestResult> {
  return invoke<BacktestResult>('run_backtest', { dslSource, symbol, initialCash });
}

// Deploy a strategy to paper or live
export async function deployStrategy(
  dslSource: string,
  name: string,
  mode: 'paper' | 'live'
): Promise<{ strategyId: string }> {
  return invoke<{ strategyId: string }>('deploy_strategy', { dslSource, name, mode });
}

// Pause or resume a running strategy
export async function setStrategyStatus(
  strategyId: string,
  status: 'running' | 'paused'
): Promise<void> {
  return invoke<void>('set_strategy_status', { strategyId, status });
}

// Get all deployed strategies
export async function listStrategies(): Promise<DeployedStrategy[]> {
  return invoke<DeployedStrategy[]>('list_strategies');
}

// Validate DSL text, returns array of error strings (empty = valid)
export async function validateDsl(dslSource: string): Promise<string[]> {
  return invoke<string[]>('validate_dsl', { dslSource });
}

// Detect if running inside Tauri
export function isTauri(): boolean {
  return typeof window !== 'undefined' && '__TAURI_INTERNALS__' in window;
}

// ----- Plugins -----
export const listPlugins = (): Promise<PluginListEntry[]> =>
  invoke<PluginListEntry[]>('list_plugins', {});
export const enablePlugin = (id: string): Promise<void> =>
  invoke<void>('enable_plugin', { id });
export const disablePlugin = (id: string): Promise<void> =>
  invoke<void>('disable_plugin', { id });
export const reloadPlugins = (): Promise<string[]> =>
  invoke<string[]>('reload_plugins', {});

// ─── Index types ───────────────────────────────────────────────────────────

export interface IndexInfo {
  alias: string;        // e.g. "NIFTY_50"
  displayName: string;  // e.g. "NIFTY 50"
  symbolCount: number;
  lastUpdated: string;  // ISO date or "never"
}

export interface RefreshResult {
  refreshed: string[];
  failed: [string, string][]; // [alias, error]
  symbolMapUpdated: boolean;
  symbolMapCount: number;
}

// ─── Index IPC wrappers ────────────────────────────────────────────────────

/** List metadata for all 22 supported indices. */
export async function listIndices(): Promise<IndexInfo[]> {
  if (!isTauri()) return [];
  return invoke<IndexInfo[]>('list_indices');
}

/** Get the constituent symbol list for a named index alias (e.g. "NIFTY_50"). */
export async function getIndexSymbols(alias: string): Promise<string[]> {
  if (!isTauri()) return [];
  return invoke<string[]>('get_index_symbols', { alias });
}

/**
 * Refresh all 22 indices from niftyindices.com and the Dhan scrip master.
 * Long-running (may take 30–60s). Show a loading state in the UI.
 */
export async function refreshIndices(): Promise<RefreshResult> {
  if (!isTauri()) {
    return { refreshed: [], failed: [], symbolMapUpdated: false, symbolMapCount: 0 };
  }
  return invoke<RefreshResult>('refresh_indices');
}

// ─── Search ───────────────────────────────────────────────────────────────

export type SymbolKind = 'equity' | 'index';

export interface SymbolMatch {
  symbol: string;
  displayName: string;
  kind: SymbolKind;
  securityId: number | null;
}

/**
 * Fuzzy-search the symbol universe (equities + 22 NSE indices). Returns
 * up to 5 ranked hits — exact match, prefix, substring, subsequence,
 * then trigram Jaccard. The browser fallback returns `[]` so the UI is
 * still demoable under `npm run dev`.
 */
export async function searchSymbols(query: string): Promise<SymbolMatch[]> {
  if (!isTauri()) return [];
  return invoke<SymbolMatch[]>('search_symbols', { query });
}
