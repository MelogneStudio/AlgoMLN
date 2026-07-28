import { invoke } from '@tauri-apps/api/core';
import { listen, type UnlistenFn } from '@tauri-apps/api/event';
import type { BacktestResult } from './backtest';
import type { DeployedStrategy } from './strategy';
import type { PluginListEntry } from './plugin';
import type {
  LiveStatusWire,
  RequestLiveStartResult,
  StopResult,
  TradeLogEntry,
} from './live';

export { listen };
export type { UnlistenFn };

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

// ─── Phase 7 — Live execution IPC ──────────────────────────────────────────

/**
 * Request a live session. Runs the preflight gates in
 * `LiveGuard::run_preflight` and returns a 90-second single-use token.
 * The UI passes the token to `confirmLiveStart` within the TTL. When
 * `requiresAck === true` the user must click "I understand" and call
 * `acknowledgeLiveTrading` before `confirmLiveStart` will succeed
 * without re-issuing the token.
 */
export async function requestLiveStart(
  strategyId: string,
): Promise<RequestLiveStartResult> {
  if (!isTauri()) {
    throw new Error('live trading is not available in the browser');
  }
  return invoke<RequestLiveStartResult>('request_live_start', { strategyId });
}

/**
 * Confirm a live session start using the token returned by
 * `requestLiveStart`. Builds and starts the `LiveSession`, fills the
 * session slot, and broadcasts the `live-session-started` lifecycle event
 * through the engine's event bus.
 */
export async function confirmLiveStart(
  strategyId: string,
  token: string,
): Promise<void> {
  if (!isTauri()) {
    throw new Error('live trading is not available in the browser');
  }
  return invoke<void>('confirm_live_start', { strategyId, token });
}

/**
 * Persist the one-time "I understand live trading is risky" consent.
 * After this succeeds, future `requestLiveStart` calls return
 * `requiresAck: false` until the ack file is removed.
 */
export async function acknowledgeLiveTrading(): Promise<void> {
  if (!isTauri()) {
    throw new Error('live trading is not available in the browser');
  }
  return invoke<void>('acknowledge_live_trading');
}

/**
 * Pause the running live session. BUY (entry) orders are suppressed; SL /
 * TP / risk-breach SELL orders always execute. Idempotent.
 */
export async function pauseLiveStrategy(): Promise<void> {
  if (!isTauri()) return;
  return invoke<void>('pause_live_strategy');
}

/** Resume a paused live session. Idempotent. */
export async function resumeLiveStrategy(): Promise<void> {
  if (!isTauri()) return;
  return invoke<void>('resume_live_strategy');
}

/**
 * Stop the live session. The session slot is cleared so a new session
 * can be started. When the stopped session left open positions, the
 * returned `openPositionsWarning` is non-null and a
 * `live-session-stopped-with-positions` Tauri event is also emitted.
 */
export async function stopLiveStrategy(): Promise<StopResult> {
  if (!isTauri()) {
    return { stopped: false, openPositionsWarning: null };
  }
  return invoke<StopResult>('stop_live_strategy');
}

/**
 * Snapshot the live session's status. Returns `null` when no session is
 * active so the UI can render an empty state.
 */
export async function getLiveStatus(): Promise<LiveStatusWire | null> {
  if (!isTauri()) return null;
  return invoke<LiveStatusWire | null>('get_live_status');
}

/**
 * Read all entries from the immutable JSONL trade log, newest first.
 * The log is append-only; entries are added on every successful order
 * placement by `DhanBroker::execute_with_meta`.
 */
export async function getTradeLog(): Promise<TradeLogEntry[]> {
  if (!isTauri()) return [];
  return invoke<TradeLogEntry[]>('get_trade_log');
}
