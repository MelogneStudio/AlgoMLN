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
