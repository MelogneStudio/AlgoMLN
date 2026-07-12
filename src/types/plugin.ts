export interface PluginVersion { major: number; minor: number; patch: number; }
export interface PluginMeta { id: string; name: string; version: PluginVersion; description: string; author: string; }
export type PluginStatus = 'Loaded' | 'Enabled' | 'Disabled' | { Failed: string };
export type Capability =
  | 'MarketData' | 'Execution' | 'Storage' | 'Events'
  | 'Indicators' | 'Analytics' | 'DslExtension' | 'UiPanels' | 'Scheduler';
export interface PluginListEntry { meta: PluginMeta; status: PluginStatus; capabilities: Capability[]; }
