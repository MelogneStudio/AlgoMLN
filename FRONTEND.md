# FRONTEND.md

Narrative on the React app — what each layer does, how state flows, and where the IPC contract is enforced. For file-by-file lookup, see `ARCHITECTURE.md`. For invariants, see `CLAUDE.md`.

---

## What the frontend does

The frontend is a single-page React 19 + TypeScript app that runs inside a Tauri v2 webview. It is intentionally a **thin client** — the Rust crate is the source of truth for execution, backtesting, and DSL validation. The React layer's job is to:

1. Hold user-facing state (builder rules, screen, modal, scale).
2. Issue IPC calls via `@tauri-apps/api::invoke`.
3. Render results.

There is no separate "live code path" in the UI either — paper and live deploys share the same screens, hooks, and wire types. The only place the Tauri boundary is felt is the `isTauri()` check in `src/types/tauri.ts`.

Live Dhan order placement is currently a backend concern behind the existing deploy/execution boundary: the React app does not build Dhan order bodies or resolve Dhan security IDs. It continues to render strategy, backtest, Settings, and IPC state while Rust owns broker execution.

```
┌────────────────────────────────────────────────────────────────────┐
│ App.tsx                                                           │
│   • scale (computed once from screen)                              │
│   • screen / modal state                                          │
│   • useStrategyBuilder (entry + exit rules)                       │
│   • useDslSync (live DSL + IPC validation)                        │
│   • useBacktest (IPC call + browser fallback)                     │
│   • backtestSymbol / backtestCapital (run config)                 │
│   • strategiesRefreshKey (bump after deploy/status)               │
└──────────────┬─────────────────────────────────────────────────────┘
               │
   ┌───────────┼────────────┬─────────────────┐
   ▼           ▼            ▼                 ▼
BuilderScreen Strategies  Settings       Modals (Coder, Uploader)
(uses dsl+backtest) (uses listStrategies)  (uses strategyToDsl,
                                            loadFromDsl, validateDsl)
```

---

## App shell and scaling

`AppWindow` (`src/components/AppWindow/AppWindow.tsx`) is the root container. It injects a single CSS variable `--ui-scale` that the entire shell scales against. Nothing in the rest of the app needs to know the actual scale number.

`lib/scaling.ts` is the single source of truth for layout:

- `DESIGN_WIDTH = 1550` and `DESIGN_HEIGHT = 757` are the fixed logical canvas dimensions. Every layout in the app assumes this canvas.
- `computeFitScale(screenW, screenH)` is called once on launch in `App.tsx`. It returns the maximum uniform scale (capped at 1.0 — never upscale) that fits the canvas with `SCREEN_PADDING = 40` of breathing room on each side.
- `applyScale(scale)` sizes the OS window to `DESIGN × scale` via `Tauri Window.setSize(LogicalSize)` and centers it. It is a no-op in the browser, so `npm run dev` works without a Tauri runtime.
- `SIDEBAR_FORCE_COLLAPSE_THRESHOLD = 0.75` — below this scale, the sidebar is **force-collapsed** and the toggle button is hidden in the title bar. This is hard-locked, not user-configurable.
- `CAPITAL_STORAGE_KEY = 'algomln_default_capital'` is the localStorage key for the default backtest capital.

There is no auto-rescale on window resize — a prior 2-second poll caused feedback loops with Tauri's `setSize` and was removed. Scale only changes via `Settings` (a slider in earlier iterations; the current build uses `computeFitScale` once at launch).

The title bar (`src/components/TitleBar/TitleBar.tsx`) is a custom one — Tauri webview's native drag is enabled via `data-tauri-drag-region` on the bar, and the optional sidebar toggle is excluded from the drag region. The OS window has decorations disabled in `tauri.conf.json` (handled by the Tauri shell), so the bar IS the title bar.

On Windows, the Tauri shell additionally applies acrylic via `window_vibrancy::apply_acrylic` and forces the WebView2 background to fully transparent so the acrylic shows through. This is set up in `src-tauri/src/main.rs`, not in the React code.

---

## Screen and modal state machine

`App.tsx` owns three pieces of routing state:

```ts
type Screen = 'builder' | 'strategies' | 'plugins' | 'settings';
type Modal  = 'none' | 'uploader' | 'coder';
```

The screen determines the main content (`BuilderScreen`, `StrategiesScreen`, `PluginsScreen`, `SettingsScreen`). The modal layers a `StrategyCoderScreen` (DSL editor) or `StrategyUploaderScreen` (file upload) on top. Only one modal at a time; clicking a different modal target swaps it.

The sidebar (`src/components/Sidebar/Sidebar.tsx`) holds a `NavItem` array of four entries: Builder, Strategies, Plugins, Settings (Plugins sits between Strategies and Settings). `onNavigate` flows back into `App.tsx` as `setScreen`. The active item gets an `aria-current="page"` and the `itemActive` style.

---

## Builder state and builder ↔ coder round-trip

`useStrategyBuilder` (`src/hooks/useStrategyBuilder.ts`) holds the visual builder's state. The shape is intentionally constrained:

```ts
interface BuilderStrategy {
  name: string;
  entry: BuilderRule;  // exactly one entry rule
  exit: BuilderRule;   // exactly one exit rule
}
```

Each `BuilderRule` is one indicator + period + comparison + rhs + action. `useStrategyBuilder` exposes `setEntryRule(patch)`, `setExitRule(patch)`, `resetStrategy`, and the round-trip function `loadFromDsl(dsl)`.

**The round-trip is the heart of the visual builder.** The flow:

1. `strategyToDsl(strategy)` in `useDslSync.ts` serializes the builder state into a `.algomln` string. The output is always:
   ```
   # Entry
   WHEN <cond>
   <action>
   
   # Exit
   WHEN <cond>
   <action>
   ```
2. `useDslSync(strategy)` derives this string with `useMemo`, then debounces a `validateDsl(dsl)` IPC call by 500ms. The debounce exists because every keystroke changes the rule shape. In the browser fallback, the hook always reports valid (the builder only emits grammars it can construct locally).
3. The builder screen renders the result. If `validationErrors.length > 0`, a toast at the bottom of `App.tsx` shows the first one and the Builder flips into **advanced mode** (`isAdvancedMode || !dslIsValid`).
4. When the user clicks "Open Strategy Coder", `App.tsx` calls `openCoderFromBuilder()`, which sets `coderSource = strategyToDsl(strategy)` and shows the editor modal. On "Done":
   - The IPC `validateDsl(source)` is called to make sure the user didn't break anything.
   - `loadFromDsl(source)` tries to parse it back via `parseDslToStrategy`.
   - If parse fails (multiple rules, AND/OR, `cross_above`/`cross_below`, anything the builder can't represent), the hook flips `isAdvancedMode = true` and shows a friendly error toast. The user stays in the coder.
   - If parse succeeds, the builder state is replaced and the modal closes.

`parseDslToStrategy` is a hand-rolled parser in `useDslSync.ts` — it deliberately only handles the subset the builder can represent and returns `null` for anything more complex. The visual builder never silently truncates a complex strategy; it pushes the user to the coder where they can keep editing.

`StrategyUploaderScreen` is the file-upload path: it reads a `.algomln` from disk, then sets `coderSource` and opens the coder in edit mode — same flow as the builder-open-coder path, but the source came from a file instead of `strategyToDsl`.

---

## The screens

### Builder (`src/screens/Builder/BuilderScreen.tsx`)

The main screen. Shows two rule sections (Entry, Exit), each a vertical list of `RuleRow` components. Each `RuleRow` is a single indicator comparison: indicator picker, period number input, op slider (`<` / `=` / `>`), RHS mode (LTP vs Value) + sign (`+` / `-`) + value, and an action line (Buy/Sell + quantity + action mode).

Below the rules is a run-config row (symbol input, capital input, Reset button) and the run action row (Backtest, Deploy). Below that, the `BacktestPanel` renders the result.

When `isAdvancedMode` is true, a yellow notice above the rules reminds the user that the visual builder can't represent the current strategy and points them at the coder.

### Strategies (`src/screens/Strategies/StrategiesScreen.tsx`)

Lists deployed strategies. Calls `listStrategies()` (Tauri IPC) on mount and on `refreshKey` change. In the browser fallback (no Tauri), renders `DEMO_STRATEGIES` (a hardcoded constant) so `npm run dev` is still demoable.

Each `StrategyCard` shows the strategy's name, description, total P&L, total trades, mode chips, status (running/paused), and a "View Code" button. The view-code path opens the coder in **read-only** mode with the strategy's DSL source — useful for inspecting a deploy without accidentally editing it.

The "Deploy" button on the builder calls `deployStrategy(dsl, name, mode)`. After the deploy, `bumpStrategies()` increments `strategiesRefreshKey` so the strategies screen reloads.

### Settings (`src/screens/Settings/SettingsScreen.tsx`)

The Settings screen is a grid of cards: **Connected Broker**, **Default Backtest Capital**, **Index Data**, and an **About** card. Default capital is persisted to localStorage under `algomln_default_capital` and loaded by `loadSavedCapital()` in `App.tsx`.

The **Index Data** card calls `listIndices()` on mount to fetch metadata for all 22 supported NSE indices. Each row shows the index's display name, symbol count, and last-updated date. The **Refresh Now** button (a `Button` with `variant="ghost"`) calls `refreshIndices()` — a long-running IPC (~30–60s) that re-pulls all 22 index constituents from niftyindices.com and the Dhan scrip master. While the refresh is in flight the button is disabled and shows "Refreshing…"; on completion an inline message reports the success/failure count and the index list is reloaded. The status grid is scrollable (`max-height: 320px`) so 22 rows don't blow out the card height.

### Modals

- `StrategyCoderScreen` — modal textarea editor. Tab inserts two spaces, Esc closes. Read-only mode disables editing. Errors are shown at the top. A `TRADE_IN` chip (small rounded badge) appears in the title row when the current source contains a `TRADE_IN` clause (detected via a case-insensitive regex on `^\s*TRADE_IN\s+(.+)$` — no full DSL parse on the frontend). A toolbar above the editor exposes a toggle that reveals a static comment block listing the 22 supported index aliases, the explicit-symbol syntax, and a note that multi-symbol strategies are paper/live only.
- `StrategyUploaderScreen` — file picker for `.algomln` files. On select, it routes the source into the coder (so the user can review before saving).

---

## The IPC hook pattern

Every Tauri call follows the same shape, exemplified by `useBacktest` (`src/hooks/useBacktest.ts`):

1. Local state: `result`, `isLoading`, `error`, and a `clear` action.
2. `run(...)` sets `isLoading = true`, clears `error`, calls the IPC, on success sets `result`, on error sets `error` and clears `result`, and **always** clears `isLoading` in `finally`.
3. If `!isTauri()` (running in a plain browser), the hook synthesises a benign placeholder so the UI is still demoable. In the backtest case it's an empty `BacktestResult` with the same initial cash.

All IPC wrappers live in `src/types/tauri.ts` and are thin `invoke<T>(name, args)` calls:

- `runBacktest(dslSource, symbol, initialCash) -> BacktestResult`
- `validateDsl(dslSource) -> string[]`
- `deployStrategy(dsl, name, mode) -> { strategyId }`
- `setStrategyStatus(strategyId, status) -> void`
- `listStrategies() -> DeployedStrategy[]`
- `listPlugins() -> PluginListEntry[]`
- `enablePlugin(id) -> void`
- `disablePlugin(id) -> void`
- `reloadPlugins() -> string[]` (per-plugin error messages, empty = clean)
- `listIndices() -> IndexInfo[]` (Settings → Index Data card)
- `refreshIndices() -> RefreshResult` (Settings → Refresh Now button; ~30–60s, writes both index JSON and symbol-map CSV)
- `getIndexSymbols(alias) -> string[]` (used by PortfolioEngine on the Rust side; not yet consumed by the UI)
- `isTauri()` — checks `'__TAURI_INTERNALS__' in window`.

The naming convention is camelCase on the TS side because that's what Tauri's invoke expects for argument keys.

### Plugin UI messages

Plugins communicate with the UI through a single Tauri event channel, `"plugin-ui-message"`. The Rust side subscribes a fresh `broadcast::Receiver<UiMessage>` from the `TauriUiApi` and re-emits every message on the bus. The frontend listener (`src/hooks/usePluginUiMessages.ts` or equivalent) does the inverse: one `listen<UiMessage>("plugin-ui-message", ...)` call, then dispatches on the `kind` discriminator:

| `kind` | Payload | UI action |
|---|---|---|
| `panelRegistered` | `{ id, title }` | Add a panel entry to the plugin-sidebar registry (lazy mount on first render) |
| `notification` | `{ msg, kind }` | Show a toast (info / warning / error) |
| `panelData` | `{ panelId, data }` | Forward `data` to the panel mounted under `panelId` (use a `useEffect` keyed on the data) |

The `UiMessage` interface lives in `src/types/plugin.ts` next to the other wire types so changes stay in sync.

### Plugin management UI

A `Plugins` screen (`src/screens/Plugins/PluginsScreen.tsx` + `.module.css`) shows the snapshot from `listPlugins()` — one card per plugin with its `name`, `id`, `description`, `version`, `author`, a `status` badge (`Loaded` / `Enabled` / `Disabled` / `Failed`), a row of capability chips, and a single toggle button: **Enable** (calls `enablePlugin(id)`) or **Disable** (`disablePlugin(id)`) depending on current status. A header **Reload** button calls `reloadPlugins()` and lists the returned per-plugin error strings above the plugin list. Failed plugins show their failure message and their toggle button is disabled.

State lives inline in the component (`plugins`, `loading`, `reloading`, `togglingId`, `reloadErrors`) — there is no separate `usePlugins()` hook. On mount it calls `listPlugins()` when `isTauri()`, otherwise falls back to a hardcoded `DEMO_PLUGINS` constant so `npm run dev` stays demoable. After each toggle or reload it refetches via `listPlugins()`. Card and badge styling follows the `SettingsScreen` / `Button` design tokens; enabled status uses `--text-green`, disabled/loaded use `--text-dim`, and failed/error text uses a red literal (`#c85a54`) since there is no red design token.

---

## Wire types

The wire types in `src/types/` mirror the Rust `BacktestResultWire`, `PaperTradeWire`, `DeployedStrategy`, etc. They are kept in sync by hand — when you add a field on the Rust side, add it to `BacktestResultWire` in `src/commands/strategy.rs` and the matching TS interface in `src/types/backtest.ts` or `src/types/strategy.ts`. This is invariant #6 in `CLAUDE.md`.

- `BacktestResult` and `BacktestSummary` — the Tauri-facing shape from `runBacktest`. Strict subset of the internal Rust `BacktestResult`; profile/log fields are dropped at the boundary.
- `PaperTrade` — `timestamp` is a `string` here (matching the wire serializer), `pnl` is `number | null`.
- `DeployedStrategy` — id, name, description, totalPnl, totalTrades, modes, status, dslSource. The single persisted `mode` becomes `modes: [mode]` on the wire.
- `BuilderStrategy` and `BuilderRule` — the builder's internal state, not the wire format. Includes `rhsSign` and `id` (uuid per rule) that don't make it to the DSL.
- `IndicatorKind`, `INDICATOR_DISPLAY`, `INDICATOR_ORDER` — the indicator enum and its display metadata.

---

## Styling and CSS

The app uses CSS Modules (`*.module.css`) per component, with a small set of shared design tokens (colors, fonts, spacing) loaded by `main.tsx`. The whole shell scales via the `--ui-scale` CSS variable injected by `AppWindow`.

The `Button` component (`src/components/Button/Button.tsx`) is the single primitive with three variants: `primary`, `ghost`, `code`. New variants go through `ButtonVariant` in the component and a CSS class in `Button.module.css`. Avoid inline-style buttons — they don't pick up hover/active states consistently.

---

## Things the frontend does NOT do

- It does not run a strategy itself. There is no JS-side backtest engine.
- It does not parse DSL itself. `parseDslToStrategy` in `useDslSync.ts` is a *builder-shaped* parser used only to round-trip the visual builder state — it is not a real DSL parser. The authoritative parser is in Rust.
- It does not store deployed strategies itself. The Tauri `StrategyRegistry` owns the persistence.
- It does not schedule ticks. WebSocket tick handling is in the Rust feed manager (`src/feed/manager.rs`); the React side has a `subscribe_ticks` IPC wrapper but no consumer yet.

If you find yourself wanting to add any of the above, the rule of thumb is: do it in Rust, expose a wire type, and have the React side render it.