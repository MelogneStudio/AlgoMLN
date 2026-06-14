
# AlgoMLN — Phase 3 + Phase 5 UI: Full Code Specification

## What You Are Building

The complete React + TypeScript frontend for AlgoMLN, a Tauri desktop trading application. This covers the **Strategy Builder screen** (Phase 5 visual algo builder), the **Strategy Coder screen** (raw DSL editor), the **Strategy Uploader modal**, the **Deployed Strategies screen**, and the **shared shell** (app window, sidebar, title bar). A **Settings screen** must also be built to match the design language, even though no Figma was provided for it.

The Rust backend and all Tauri IPC commands already exist. You are building the frontend only. Do not touch anything in `src-tauri/`.

---

## Tech Stack

- React 18 + TypeScript (strict mode, zero `any`)
- Tauri v2 (`@tauri-apps/api`) for IPC — use `invoke` and `listen`, never `fetch` to localhost
- CSS Modules for all styling — no Tailwind, no CSS-in-JS, no inline style objects except where a dynamic gradient is unavoidable
- No UI component libraries (no shadcn, no MUI, no Radix) — every component is hand-built from scratch to match the Figma exactly
- `@tauri-apps/plugin-dialog` for native file open dialogs

---

## File Structure

Build exactly this structure. Do not deviate.

```
src/
  main.tsx                        ← Tauri app entry, mounts <App />
  App.tsx                         ← Root: AppWindow shell + router (screen switcher)
  App.module.css

  styles/
    tokens.css                    ← ALL design tokens as CSS custom properties
    fonts.css                     ← @font-face declarations for all custom fonts
    global.css                    ← body reset, box-sizing, scrollbar styles

  components/
    AppWindow/
      AppWindow.tsx               ← The frosted glass window wrapper (1550×757px)
      AppWindow.module.css
    TitleBar/
      TitleBar.tsx                ← Drag region, window controls (minimize/screenshot/close)
      TitleBar.module.css
    Sidebar/
      Sidebar.tsx                 ← Collapsible sidebar, both collapsed (84px) and expanded (280px) states
      Sidebar.module.css
    OptionSlider/
      OptionSlider.tsx            ← Sliding pill selector (the < = > and LTP/Value controls)
      OptionSlider.module.css
    NumberInput/
      NumberInput.tsx             ← Editable number field (the period/quantity inputs)
      NumberInput.module.css
    IndicatorPicker/
      IndicatorPicker.tsx         ← SMA/EMA/RSI/etc dropdown with chevron button
      IndicatorPicker.module.css
    RuleRow/
      RuleRow.tsx                 ← One condition + action row (the "If SMA 14 is < LTP + Buy 20 Quantity" row)
      RuleRow.module.css
    Button/
      Button.tsx                  ← Primary button (Backtest, Deploy, Done, Pause)
      Button.module.css

  screens/
    Builder/
      BuilderScreen.tsx           ← Strategy Builder main screen
      BuilderScreen.module.css
      components/
        RuleSection.tsx           ← The Entry or Exit panel (green/red tinted container)
        RuleSection.module.css
        BacktestPanel.tsx         ← The Overview + Backtest results panel (scrolls below fold)
        BacktestPanel.module.css
    StrategyCoder/
      StrategyCoderScreen.tsx     ← Modal overlay: code editor + Done button
      StrategyCoderScreen.module.css
    StrategyUploader/
      StrategyUploaderScreen.tsx  ← Modal overlay: drag-drop + file picker + Open Editor
      StrategyUploaderScreen.module.css
    Strategies/
      StrategiesScreen.tsx        ← Deployed Strategies list screen
      StrategiesScreen.module.css
      components/
        StrategyCard.tsx          ← One deployed strategy card
        StrategyCard.module.css
    Settings/
      SettingsScreen.tsx          ← Settings screen (no Figma — infer from design language)
      SettingsScreen.module.css

  types/
    strategy.ts                   ← All TypeScript types for strategy state
    backtest.ts                   ← BacktestResult, BacktestSummary types
    tauri.ts                      ← Typed wrappers for all invoke calls

  hooks/
    useStrategyBuilder.ts         ← All builder state logic
    useBacktest.ts                ← Backtest invocation + result state
    useDslSync.ts                 ← Converts builder state → DSL string (explained below)
```

---

## Design Tokens (`styles/tokens.css`)

Define every value as a CSS custom property. Do not hardcode any color, blur, or shadow anywhere else.

```css
:root {
  /* Backgrounds */
  --app-window-bg: rgba(34, 34, 34, 0.6);
  --mainframe-outframe: rgba(34, 34, 34, 0.4);
  --entry-bg: rgba(8, 255, 0, 0.05);
  --exit-bg: rgba(255, 0, 0, 0.07);
  --rule-inner-bg: rgba(17, 17, 17, 0.4);
  --option-bg: rgba(51, 51, 51, 0.6);
  --highlight: #555555;
  --sidebar-bg: rgba(0, 0, 0, 0.3);
  --titlebar-controls-bg: rgba(34, 34, 34, 0.7);
  --code-editor-bg: rgba(17, 17, 17, 0.8);
  --modal-overlay-bg: rgba(0, 0, 0, 0.3);
  --upload-drop-bg: rgba(102, 102, 102, 0.5);
  --strategy-card-bg: rgba(34, 34, 34, 0.4);

  /* Accent gradients (applied as background-image, not background-color) */
  --accent-gradient: linear-gradient(90deg, rgba(145, 133, 67, 0.2) 0%, rgba(145, 133, 67, 0.2) 100%),
                     linear-gradient(90deg, rgba(67, 145, 84, 0.3) 0%, rgba(67, 145, 84, 0.3) 100%);
  --accent-gradient-hover: linear-gradient(90deg, rgba(255, 239, 148, 0.2) 0%, rgba(255, 239, 148, 0.2) 100%),
                           linear-gradient(90deg, rgba(163, 226, 177, 0.3) 0%, rgba(163, 226, 177, 0.3) 100%);
  --entry-gradient: linear-gradient(90deg, rgba(8, 255, 0, 0.05) 0%, rgba(8, 255, 0, 0.05) 100%),
                    linear-gradient(90deg, rgba(34, 34, 34, 0.4) 0%, rgba(34, 34, 34, 0.4) 100%);
  --exit-gradient: linear-gradient(90deg, rgba(255, 0, 0, 0.07) 0%, rgba(255, 0, 0, 0.07) 100%),
                   linear-gradient(90deg, rgba(34, 34, 34, 0.4) 0%, rgba(34, 34, 34, 0.4) 100%);

  /* Text */
  --text-primary: #ffffff;
  --text-muted: rgba(255, 255, 255, 0.7);
  --text-dim: #777777;
  --text-green: #5ea94d;
  --text-yellow: #98a94d;

  /* Borders */
  --border-option: #444444;
  --border-rule: rgba(119, 119, 119, 0.5);
  --border-modal: rgba(127, 127, 127, 0.6);
  --border-sidebar-selected: #d6d6d6;
  --border-upload: rgba(119, 119, 119, 0.4);

  /* Shadows */
  --shadow-sidebar: 0px 4px 15px 5px rgba(255, 255, 255, 0.25);
  --shadow-sidebar-selected: 0px 0px 5px 4px rgba(255, 255, 255, 0.2);
  --shadow-logo: 0px 0px 15px 0px white;
  --shadow-rule-inner: 0px 0px 15px 10px rgba(255, 255, 255, 0.05);
  --shadow-modal: 0px 0px 15px 10px rgba(255, 255, 255, 0.2);
  --shadow-app-window: blur(40px);

  /* Status badge colors */
  --badge-paper: #649f79;
  --badge-live: #649f79;

  /* Radii */
  --radius-app: 20px;
  --radius-sidebar: 40px;
  --radius-sidebar-expanded: 60px;
  --radius-card: 40px;
  --radius-rule: 40px;
  --radius-option: 20px;
  --radius-option-inner: 16px;
  --radius-pill: 999px;
  --radius-badge: 999px;

  /* Dimensions */
  --app-width: 1550px;
  --app-height: 757px;
  --sidebar-collapsed-width: 84px;
  --sidebar-expanded-width: 280px;
  --titlebar-height: 50px;
}
```

---

## Fonts (`styles/fonts.css`)

These are the exact fonts used. Load them from Google Fonts or a local `public/fonts/` folder.

```css
/* Sora — titles */
/* Sansation — body labels */
/* Cascadia Code — monospace options/numbers */
/* Rajdhani — section headers (Entry/Exit) */
/* Geist Mono — code editor */
/* Phudu — strategy card titles */
/* Inter — lambda logo character */
```

---

## TypeScript Types (`types/strategy.ts`)

```typescript
// Every indicator the DSL supports
export type IndicatorKind =
  | 'sma' | 'ema' | 'rsi' | 'atr' | 'vwap'
  | 'bb_upper' | 'bb_lower' | 'bb_mid';

// Price fields usable as the right-hand side
export type PriceField = 'close' | 'open' | 'high' | 'low' | 'volume';

// Comparison operators
export type CompareOp = '<' | '=' | '>';

// What the right side of a condition can be
export type RhsMode = 'ltp' | 'value';  // 'ltp' maps to close price, 'value' is a literal number

// +/- for the right-hand side modifier
export type RhsSign = '+' | '-';

// Entry action mode
export type ActionMode = 'quantity' | 'money';

// Sell mode (only used for Exit)
export type SellMode = 'quantity' | 'money' | 'all';

// One complete condition + action row (matches what the Figma builder shows)
export interface BuilderRule {
  id: string;                    // uuid, stable across re-renders

  // Condition (left side)
  indicator: IndicatorKind;
  period: number;                // e.g. 14 for RSI(14)
  op: CompareOp;

  // Condition (right side)
  rhsMode: RhsMode;
  rhsSign: RhsSign;
  rhsValue: number;              // the literal threshold, e.g. 70

  // Action
  actionVerb: 'buy' | 'sell';   // fixed: entry rules always buy, exit rules always sell
  actionMode: ActionMode | SellMode;
  actionQuantity: number;        // shares or money amount
}

// The full strategy as the builder holds it
export interface BuilderStrategy {
  name: string;
  entry: BuilderRule;            // exactly one entry rule in v1
  exit: BuilderRule;             // exactly one exit rule in v1
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
  dslSource: string;             // the raw .algomln text
}
```

---

## TypeScript Types (`types/backtest.ts`)

```typescript
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

export interface BacktestResult {
  tradeHistory: PaperTrade[];
  finalCash: number;
  initialCash: number;
  totalRealizedPnl: number;
  totalCandlesProcessed: number;
  summary: BacktestSummary;
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
```

---

## Tauri IPC Wrappers (`types/tauri.ts`)

```typescript
import { invoke } from '@tauri-apps/api/core';
import type { BacktestResult } from './backtest';
import type { DeployedStrategy } from './strategy';

// Run a backtest by passing raw DSL text
export async function runBacktest(
  dslSource: string,
  symbol: string,
  initialCash: number
): Promise<BacktestResult> {
  return invoke('run_backtest', { dslSource, symbol, initialCash });
}

// Deploy a strategy to paper or live
export async function deployStrategy(
  dslSource: string,
  name: string,
  mode: 'paper' | 'live'
): Promise<{ strategyId: string }> {
  return invoke('deploy_strategy', { dslSource, name, mode });
}

// Pause or resume a running strategy
export async function setStrategyStatus(
  strategyId: string,
  status: 'running' | 'paused'
): Promise<void> {
  return invoke('set_strategy_status', { strategyId, status });
}

// Get all deployed strategies
export async function listStrategies(): Promise<DeployedStrategy[]> {
  return invoke('list_strategies');
}

// Validate DSL text, returns array of error strings (empty = valid)
export async function validateDsl(dslSource: string): Promise<string[]> {
  return invoke('validate_dsl', { dslSource });
}
```

---

## DSL Translation (`hooks/useDslSync.ts`)

This is the critical bridge between the visual builder and the Rust engine. The hook takes the current `BuilderStrategy` state and computes the equivalent `.algomln` DSL string in real-time. This string is what gets passed to `runBacktest` and `deployStrategy`.

**Translation rules — implement these exactly:**

```typescript
// Indicator DSL name mapping
const INDICATOR_DSL: Record<IndicatorKind, string> = {
  sma: 'ma',          // Figma shows "SMA", DSL keyword is "ma"
  ema: 'ema',
  rsi: 'rsi',
  atr: 'atr',
  vwap: 'vwap',
  bb_upper: 'bb_upper',
  bb_lower: 'bb_lower',
  bb_mid: 'bb_mid',
};

// Operator mapping
const OP_DSL: Record<CompareOp, string> = {
  '<': '<',
  '=': '==',     // the single "=" in the UI slider maps to "==" in DSL
  '>': '>',
};

// RHS translation
// If rhsMode === 'ltp': right side is the keyword "close"
// If rhsMode === 'value': right side is the literal rhsValue number
// rhsSign only applies when rhsMode === 'value': it prefixes the number (but since DSL
// has no signed literals, represent negative with: close - N / close + N ... actually
// in v1 the sign modifier is visual only and the right side is always the plain number)

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

// Full strategy → DSL string
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
```

The hook exports:
- `dsl: string` — the live DSL string, updated on every state change
- `isValid: boolean` — derived from validateDsl (debounced 500ms)
- `validationErrors: string[]`

---

## Hook: `useStrategyBuilder.ts`

```typescript
// Initial state — matches exactly what's shown in the Figma
const DEFAULT_ENTRY_RULE: BuilderRule = {
  id: crypto.randomUUID(),
  indicator: 'sma',
  period: 14,
  op: '<',
  rhsMode: 'ltp',
  rhsSign: '+',
  rhsValue: 70,
  actionVerb: 'buy',
  actionMode: 'quantity',
  actionQuantity: 20,
};

const DEFAULT_EXIT_RULE: BuilderRule = {
  id: crypto.randomUUID(),
  indicator: 'sma',
  period: 14,
  op: '<',
  rhsMode: 'ltp',
  rhsSign: '+',
  rhsValue: 70,
  actionVerb: 'sell',
  actionMode: 'quantity',
  actionQuantity: 20,
};

// Exports:
// strategy: BuilderStrategy
// setEntryRule: (patch: Partial<BuilderRule>) => void
// setExitRule: (patch: Partial<BuilderRule>) => void
// resetStrategy: () => void
// loadFromDsl: (dsl: string) => void   ← for when user comes back from the coder
```

---

## Hook: `useBacktest.ts`

```typescript
// Exports:
// runBacktest: (symbol: string, cash: number) => Promise<void>
// result: BacktestResult | null
// isLoading: boolean
// error: string | null
// clearResult: () => void
```

---

## App Shell (`App.tsx`)

The app has a single window that never navigates away — it switches screens by swapping the content area. No React Router needed.

```typescript
type Screen = 'builder' | 'strategies' | 'settings';

// The modal layer sits above all screens
type Modal = 'none' | 'uploader' | 'coder';
```

State lives in `App.tsx` and flows down as props. Do not use global state management (no Redux, no Zustand).

---

## AppWindow Component

The outer frosted glass window. Always 1550×757px, centered on the OS desktop background (the wallpaper is rendered behind it — don't try to render the wallpaper, Tauri handles that). It has:
- `backdrop-filter: blur(40px)` on the window itself
- `background: var(--app-window-bg)`
- `border-radius: var(--radius-app)`
- `overflow: hidden` (clips children)
- Contains `<TitleBar />` pinned at the top, then the content area, then `<Sidebar />` absolutely positioned on the left

---

## TitleBar Component

Height: `var(--titlebar-height)` = 50px. Pinned at the top.

Left side: Sidebar toggle button — `84×40px`, `border-radius: 20px`, background `rgba(34,34,34,0.8)`. Contains the sidebar icon. Clicking it toggles sidebar expanded/collapsed. Use `data-tauri-drag-region` on the drag area.

Right side: Window controls group — `178×45px`, `border-radius: 20px`, background `rgba(34,34,34,0.7)`. Contains three buttons: minimize, screenshot (camera icon), close. Use Tauri's window API:
```typescript
import { getCurrentWindow } from '@tauri-apps/api/window';
const win = getCurrentWindow();
// minimize: win.minimize()
// close: win.close()
// screenshot: win.capture() or similar — if unavailable, make it a no-op
```

---

## Sidebar Component

Two visual states: **collapsed** (84px wide) and **expanded** (280px wide). The toggle comes from `App.tsx`.

### Collapsed state
Height: 650px. Position: absolute, left 31px, top 54px. `border-radius: 40px`. `backdrop-filter: blur(45px)`. Background: `var(--sidebar-bg)`. Box shadow: `var(--shadow-sidebar)`.

Nav items (top to bottom, centered horizontally):
1. **Logo button** — at top (y=51). 56px wide, 56px tall. Background: `var(--accent-gradient)`. Box shadow: `var(--shadow-logo)`. Shows `λ` in Inter 45px at 70% white opacity.
2. **Builder icon** — y=190. When active: `var(--accent-gradient-hover)`, white border `1px`, box shadow `0px 0px 4px 5px rgba(255,255,255,0.2)`.
3. **Strategies icon** — y=297. When active: same highlight treatment.
4. **Settings icon** — y=404. When active: same.
5. **Separator line** — horizontal, at y≈568.
6. **Profile avatar** — y=584. Round avatar, 56×56px. Contains a 50×50 circular avatar image.

### Expanded state
Width: 280px. Same height and position. `border-radius: 60px`. Each nav item is now 200×50px and shows icon + text label. Text in Sansation Bold 28px, `var(--text-muted)`. The logo item is 204px wide and shows `λlgoMLN`.

Active item: `var(--accent-gradient-hover)`, white border, glow shadow.

The sidebar must animate between collapsed and expanded — use a CSS transition on `width` and fade text in/out (opacity transition on labels).

---

## OptionSlider Component

This is the sliding pill selector used throughout the builder. It takes an array of options and a selected index, and renders a sliding highlight under the active option.

```typescript
interface OptionSliderProps {
  options: string[];        // e.g. ['<', '=', '>'] or ['LTP', 'Value'] or ['+', '-']
  selectedIndex: number;
  onChange: (index: number) => void;
  width?: number;           // total width in px
  height?: number;          // total height in px (default 50)
}
```

- Outer: `border-radius: var(--radius-option)`, `border: 2px solid var(--border-option)`, `background: var(--option-bg)`
- Active pill: `background: var(--highlight)`, `border-radius: var(--radius-option-inner)`, positioned 3px from edges with `left: calc(selectedIndex * slotWidth + 3px)`, CSS `transition: left 150ms ease`
- Inactive option text: `color: var(--text-dim)`
- Active option text: `color: var(--text-muted)`
- Font: Cascadia Code Bold 28px

---

## NumberInput Component

Editable number. Shows the value; clicking enters edit mode (becomes a real `<input type="number">`). On blur or Enter, commits.

```typescript
interface NumberInputProps {
  value: number;
  onChange: (value: number) => void;
  min?: number;
  max?: number;
  width?: number;
}
```

- Style: `border-radius: 15px`, `border: 2px solid var(--border-option)`, `background: var(--option-bg)`, height 50px
- Font: Cascadia Code Bold 28px, `color: var(--text-muted)`, centered
- In edit mode, `<input>` inherits same style, no native browser chrome

---

## IndicatorPicker Component

Dropdown for selecting the indicator. Shows the current indicator name (e.g. "SMA") on the left and a chevron-down button on the right inside a highlighted box.

```typescript
interface IndicatorPickerProps {
  value: IndicatorKind;
  onChange: (value: IndicatorKind) => void;
  width?: number;
}
```

Options: SMA, EMA, RSI, ATR, VWAP, BB Upper, BB Lower, BB Mid.

- The chevron button is on the right: 40×40px, `border-radius: 10px`, `background: var(--highlight)`, contains a ▾ character or SVG arrow
- Clicking opens a dropdown list absolutely positioned below. Dropdown: `background: rgba(34,34,34,0.95)`, `border-radius: 16px`, `border: 2px solid var(--border-option)`
- Each option in the list: Cascadia Code Bold 28px, hoverable with subtle highlight

---

## RuleRow Component

One complete row: `If [IndicatorPicker] [period: NumberInput] is [operator: OptionSlider] [rhs: composite]` and on a second line `Buy/Sell [qty: NumberInput] [actionMode: OptionSlider]`.

```typescript
interface RuleRowProps {
  rule: BuilderRule;
  onChange: (patch: Partial<BuilderRule>) => void;
  isExitRule: boolean;    // exit rules show "Sell" and have the "All" option + hint text
}
```

**Condition line layout (left to right):**
- "If" label — Sansation Bold 28px white, left ~32px
- `IndicatorPicker` — x≈63, width 164px
- `NumberInput` (period) — x≈249, width 101px
- "is" label — Sansation Bold 28px white, x≈383
- `OptionSlider` (operator: `<` / `=` / `>`) — x≈416, width 185px
- RHS composite block — x≈623: contains `OptionSlider` (LTP/Value, width 200px) + `OptionSlider` (+/-, width 107px) + literal value display (the 70 number)

**Action line layout (left to right):**
- "Buy" or "Sell" label — Cascadia Code Bold 28px white, x≈48, y≈103
- `NumberInput` (quantity) — x≈95, width 200px
- `OptionSlider` (Quantity/Money/All) — x≈317. Entry has [Quantity, Money]. Exit has [Quantity, Money, All].

**Exit rule only:** show hint text "won't sell if no holdings" in Sansation Bold 20px `var(--text-dim)`, bottom right of the inner box.

---

## RuleSection Component

The green (Entry) or red (Exit) tinted container that wraps a `RuleRow`.

```typescript
interface RuleSectionProps {
  type: 'entry' | 'exit';
  rule: BuilderRule;
  onChange: (patch: Partial<BuilderRule>) => void;
}
```

- Outer: `border-radius: 40px`, padding 15px top/bottom 10px. Background: `var(--entry-gradient)` or `var(--exit-gradient)`
- Label above the inner box: Rajdhani SemiBold 30px white, tracking 0.3px. "Entry" or "Exit"
- Inner box: `background: var(--rule-inner-bg)`, `border: 2px solid var(--border-rule)`, `border-radius: 40px`, height 166px, box shadow `var(--shadow-rule-inner)`

---

## BuilderScreen

Layout (all within the app content area, left of sidebar):

```
[TitleBar — top]
[Page title "Strategy Builder" — y=73, left=158, in a --mainframe-outframe pill]
["Open Strategy Coder" button — top right, y=79, left=1101]

[Entry RuleSection — vertically centered at 50% - 122px]
[Exit RuleSection — vertically centered at 50% + 109px]

[Backtest + Deploy buttons — y=625, centered]

[BacktestPanel — below, starts at y=669, scrolls]
```

**"Open Strategy Coder" button:** `border: 4px solid var(--border-modal)`, `border-radius: var(--radius-pill)`, background `var(--mainframe-outframe)`. Contains code icon + "Open Strategy Coder" text in Cascadia Code Bold 24px `var(--text-muted)`. Clicking opens the `StrategyCoder` modal.

**Backtest button:** `border-radius: var(--radius-pill)`, background `var(--accent-gradient)`. Play icon + "Backtest" text in Cascadia Code Bold 32px `var(--text-muted)`. Clicking calls `runBacktest` with the live DSL.

**Deploy button:** Same style. Upload icon + "Deploy" text. Clicking opens the `StrategyUploader` modal.

---

## BacktestPanel Component

Scrolls below the builder (appears when the window is scrolled or after a backtest runs). Two sections:

**Overview card:** `border-radius: 40px`, background `var(--mainframe-outframe)`. Grid layout, 2 columns:
```
Net PnL :    [value]
Max Loss:    [value]
Max Profit   [value]
```
Labels: Sansation Bold 28px white. Values: Sansation Bold 28px `var(--text-dim)`. Show "—" before a backtest runs, actual values after.

**Backtest results card:** `border-radius: 40px`, background `var(--mainframe-outframe)`. When no data: centered text "Oops! No data found" in Sansation Bold 28px `var(--text-dim)`. When data exists: render a scrollable table of trades (timestamp, side, price, qty, pnl per row) in Cascadia Code 24px. PnL positive values in `var(--text-green)`, negative in red `#e05252`.

---

## StrategyCoderScreen (modal)

A full-window modal overlay: `backdrop-filter: blur(15px)`, `background: var(--modal-overlay-bg)`, `border: 4px solid var(--border-modal)`. Sits on top of everything.

Inner card: `background: rgba(34,34,34,0.7)`, `border-radius: 80px`, `box-shadow: var(--shadow-modal)`. Width ~1297px, centered. Contains:

1. **Title** "Code your strategy" — Sora SemiBold 40px white, centered
2. **Code editor area** — `background: var(--code-editor-bg)`, `border-radius: 40px`, height 370px. Contains:
   - File tab pill top-left: `strategy.algomln` in Geist Mono Bold 24px white + pencil icon, background `rgba(102,102,102,0.5)`, `border-radius: 999px`
   - A real `<textarea>` for editing. Font: Geist Mono Bold 24px white. Background transparent. No border. Padding: 87px top, 41px left. Resize: none. The textarea fills the editor area. Tab inserts 2 spaces (intercept keydown Tab).
3. **Done button** — `border-radius: 999px`, background `var(--accent-gradient)`. Check icon + "Done" text in Sansation Bold 28px white. Clicking: validates the DSL, if valid closes modal and calls `loadFromDsl` to sync the builder state back from the code, if invalid shows inline error.

**Sync direction when closing the coder:** Parse the textarea content back into `BuilderStrategy` by calling `validateDsl`. If valid, call `loadFromDsl(dslText)` which updates the builder. The builder UI then reflects the code.

**Simple DSL → BuilderStrategy parser for `loadFromDsl`:** Parse only the supported subset (single WHEN/BUY and WHEN/SELL pattern). If the DSL is more complex than the builder can represent, keep the DSL text but mark the builder as "advanced mode" (show a notice instead of the controls, with a link to reopen the coder). This prevents data loss.

---

## StrategyUploaderScreen (modal)

Same overlay style as the coder modal. Inner card: `border-radius: 80px`, width ~812px, height ~600px.

Contains:
1. **Title** "Upload your strategy" — Sora SemiBold 40px white
2. **Drag-and-drop zone** — `background: var(--upload-drop-bg)`, `border: 3px solid var(--border-upload)`, `border-radius: 40px`. Archive icon centered + text "Drag and drop your .algomln strategy here" in Sansation Bold 28px white. Implement real drag-and-drop:
   - `onDragOver`: prevent default, add `.dragging` class for visual feedback (lighten border)
   - `onDrop`: read the file, call `loadFromDsl`
3. **"Or choose a file" button** — `background: var(--upload-drop-bg)`, `border: 3px solid var(--border-upload)`, `border-radius: 40px`. Download-file icon + text. Clicking: use `@tauri-apps/plugin-dialog` `open({ filters: [{ name: 'AlgoMLN Strategy', extensions: ['algomln'] }] })`, read file via `@tauri-apps/plugin-fs` `readTextFile`, call `loadFromDsl`.
4. **"Don't have a strategy file?" row** — text + "Open Editor" button with code icon. Clicking closes this modal and opens the coder modal instead.

---

## StrategiesScreen

Deployed strategies list. Shows `<StrategyCard />` for each item returned by `listStrategies()`. Calls `listStrategies` on mount with a loading skeleton while fetching.

---

## StrategyCard Component

Matches the Figma: `background: var(--strategy-card-bg)`, `border-radius: 40px`, height 194px, width 1294px, left 158px.

Layout:
- **Top row:** Strategy name in Phudu Regular 30px white + edit pencil icon. To the right: description in Cascadia Code Bold 28px `rgba(255,255,255,0.6)`. Far right: mode badges. Each badge: `border-radius: 999px`, background `var(--badge-paper)`, Cascadia Code Bold 20px white, padding 3px 20px.
- **Middle row:** "Total PnL: [value]" — Cascadia Code Bold 28px. Positive value in `var(--text-green)`. "Total Trades: [count]" in `var(--text-yellow)`.
- **Bottom row left:** Pause/Resume button — `border-radius: 999px`, background `var(--accent-gradient)`, pause icon + "Pause" or play icon + "Resume" text. Clicking calls `setStrategyStatus`.
- **Bottom row right:** "View Code" button — `border: 4px solid var(--border-modal)`, `border-radius: 20px`, background `var(--mainframe-outframe)`, code icon + "View Code". Clicking opens the coder modal with `dslSource` loaded as read-only (or editable — user can choose).

---

## SettingsScreen

No Figma was provided. Build this to match the visual language of all other screens precisely.

Structure:
- Same page title pill as other screens: "Settings" in Sora SemiBold 40px
- Three settings cards in `var(--mainframe-outframe)`, `border-radius: 40px`, each ~400px wide

**Card 1 — Broker:**
- Label: Sansation Bold 28px white: "Connected Broker"
- Value: Cascadia Code Bold 28px `var(--text-muted)`: "Dhan (API Key set)"
- Subtext: "Upstox support coming soon" in `var(--text-dim)` 20px

**Card 2 — Default Capital:**
- Label: "Default Backtest Capital"
- A `NumberInput`-style field for the default starting cash (₹100,000 default)
- Persists to `localStorage`

**Card 3 — About:**
- "AlgoMLN" in Sora SemiBold 40px white
- Version string: "v0.1.0" in Cascadia Code 28px `var(--text-dim)`
- "Built for algo traders, by algo traders." in Sansation Bold 28px `var(--text-muted)`

---

## Bug-Prevention Rules

These are non-negotiable. Follow every one.

**State rules:**
- `BuilderStrategy` state lives only in `useStrategyBuilder`. Never duplicate it in screen-level state.
- The DSL string is always derived (computed) — never stored as state alongside `BuilderStrategy`. `useDslSync` derives it on every render.
- Rule IDs are `crypto.randomUUID()` assigned once at creation and never regenerated.

**Render rules:**
- `OptionSlider` must never reposition the active pill by re-mounting — use CSS `left` transition only.
- `NumberInput` must commit its value on blur AND on Enter. It must not fire `onChange` on every keystroke — only on commit.
- `IndicatorPicker` dropdown must close on outside click (use a `useEffect` with `document.addEventListener('mousedown', ...)` that checks `!ref.current.contains(event.target)`).

**DSL translation rules:**
- The `=` operator in the UI slider maps to `==` in DSL — not `=`. This is the most common source of parse errors. Test this explicitly.
- `SMA` in the UI maps to `ma` in DSL (the indicator function is `ma`, not `sma`). Wrong mapping = silent incorrect backtest.
- When `SellMode === 'all'`, the DSL must emit `SELL ALL`, not `SELL 0` or `SELL 20`.
- DSL is always uppercase keywords: `WHEN`, `BUY`, `SELL`, `ALL`. Indicator function names are always lowercase: `ma(14)`, `rsi(14)`.

**Tauri IPC rules:**
- All `invoke` calls must be wrapped in try/catch. Errors go to a toast or inline error state — never silently swallowed.
- Never call `invoke` inside a render function. Always call inside event handlers or `useEffect`.
- File reading must use `@tauri-apps/plugin-fs`, not the browser `fetch` or `FileReader` APIs.

**CSS rules:**
- Never use `position: absolute` for layout that can be done with flexbox.
- All pixel values come from tokens — do not hardcode `rgba()` or `px` values inside component CSS modules if a token exists.
- The app window must never have a scrollbar on the X axis. `overflow-x: clip` on `AppWindow`.
- The `backdrop-filter` on the sidebar will not work on Windows if the sidebar has `overflow: hidden` — use `overflow: clip` instead (CSS, not the property value `hidden`).

**Accessibility minimums:**
- All interactive elements must have a visible focus ring (use `outline: 2px solid rgba(255,255,255,0.4)` on `:focus-visible`).
- The drag-and-drop zone must also be keyboard-activatable (Enter/Space triggers the file picker).
- Buttons must have `aria-label` when they contain only an icon.

---

## Indicator Display Name Map

Used in the UI only. Not the DSL name.

```typescript
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
```

---

## What You Must Not Do

- Do not install Tailwind, shadcn, Radix, or any UI kit.
- Do not use `any` in TypeScript.
- Do not put logic inside `.module.css` files — logic belongs in hooks and components.
- Do not call the Tauri IPC from inside CSS or render paths — only from event handlers or effects.
- Do not create a separate process or localhost server. All backend communication is Tauri IPC only.
- Do not skip the `loadFromDsl` round-trip when closing the coder modal. Silent state divergence between the code editor and the visual builder is the most dangerous bug in this screen.
- Do not hardcode the DSL operator `=` where the DSL requires `==`.
- Do not render the OS wallpaper — Tauri renders it behind the transparent window.