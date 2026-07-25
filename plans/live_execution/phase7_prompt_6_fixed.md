# AlgoMLN — Phase 7, Prompt 6 (Fixed): Live Trading UI

Run this prompt in Claude Code / Cursor / Windsurf against the existing repo.
This is the fixed version of Phase 7 Prompt 6.

Output code only — no explanations, no questions, unless explicitly told to ask.

---

## Environment note (read first)

This machine does **not** have Rust, Node, or npm installed. You cannot
run `npm run build`, `npm run dev`, `cargo build`, or any test command
yourself. Whenever you would normally run one:

1. Write the code change first.
2. State the exact command you want executed.
3. Ask the user to run it and paste back the full output.
4. Wait for the results before moving on.
5. If a command fails, ask for the complete error text — do not guess.

Do not attempt to install toolchains.

---

## What already exists (context for a fresh AI)

Prompts 1–5 of Phase 7 (with the fix pack for 1–3, and the fixed 4 and 5)
have been implemented. You can assume:

- Tauri commands: `request_live_start`, `confirm_live_start`,
  `acknowledge_live_trading`, `pause_live_strategy`,
  `resume_live_strategy`, `stop_live_strategy`, `get_live_status`,
  `get_trade_log`.
- Frontend wrappers in `src/types/tauri.ts` matching those.
- Types in `src/types/live.ts`: `RequestLiveStartResult`, `StopResult`,
  `LiveStatusWire`, `TradeLogEntry`, `LiveSessionFailedPayload`,
  `LiveSessionStoppedPayload`.
- Tauri events: `live_session_failed`, `live_session_stopped_with_positions`.
- Design tokens: dark green-black palette, `--text-green`, `--text-dim`,
  Cascadia Code, CSS Modules, no external UI component libraries.
- Existing screens: `Builder`, `Strategies`, `Plugins`, `Settings`. The
  sidebar currently has four nav items.
- `StrategyCard` in `StrategiesScreen` — each rendered card. Live-mode
  strategies have `mode == 'live'`.
- `LiveStatusWire.realizedLoss` is a **non-negative magnitude**. Losses
  are always ≥ 0; there is no such thing as a "negative loss" here.

If any of the above is missing, stop and ask.

---

## Correct ack flow (important)

`requestLiveStart` returns `{ token, requiresAck }` — the token is
issued whether or not ack is required. The UI flow:

1. User clicks "Go Live" → open modal (Step 1: Pre-flight).
2. User clicks "Run Checks" → call `requestLiveStart(strategyId)`.
3. If it returns `{ requiresAck: true, token }`: show the ack modal.
   On "I Understand", call `acknowledgeLiveTrading()`, then advance to
   Step 2 with the token already in hand.
4. If it returns `{ requiresAck: false, token }`: advance directly to
   Step 2 with the token.
5. Step 2: 3-second countdown, then "Confirm — Go Live" calls
   `confirmLiveStart(strategyId, token)`.
6. On any gate failure at step 2, the token is one-use and now invalid.
   The user must go back to Step 1 and click "Run Checks" again.

The token has a 90-second TTL. The Step 2 countdown consumes 3 seconds
of that; a slow reader has plenty of room.

---

## Files to create/edit

### A. `src/hooks/useLiveStatus.ts` — new hook

```typescript
import { useCallback, useEffect, useRef, useState } from "react";
import { getLiveStatus } from "../types/tauri";
import type { LiveStatusWire } from "../types/live";

export function useLiveStatus(pollIntervalMs = 5000) {
    const [status, setStatus] = useState<LiveStatusWire | null>(null);
    const [error, setError] = useState<string | null>(null);
    const timerRef = useRef<number | null>(null);

    const fetchOnce = useCallback(async () => {
        try {
            const s = await getLiveStatus();
            setStatus(s);
            setError(null);
        } catch (e) {
            setError(String(e));
        }
    }, []);

    useEffect(() => {
        fetchOnce();
        timerRef.current = window.setInterval(fetchOnce, pollIntervalMs);
        return () => {
            if (timerRef.current !== null) {
                window.clearInterval(timerRef.current);
                timerRef.current = null;
            }
        };
    }, [fetchOnce, pollIntervalMs]);

    return { status, error, refresh: fetchOnce };
}
```

Use this hook everywhere status is needed. Do not add ad-hoc
`setInterval` polls elsewhere.

### B. `src/screens/Live/LiveScreen.tsx` + `LiveScreen.module.css`

Four sections, vertically stacked, using card layouts consistent with
`StrategiesScreen`.

**Section 1 — Status Card** (always visible):
- Header: "Live Trading". When `status.status === 'Running'`, show a
  pulsing green dot: CSS animation, 1s ease-in-out infinite alternate,
  opacity 0.3 → 1.0, `border-radius: 50%`,
  `background: var(--text-green)`.
- If `status == null`: centered text "No live strategy running." in
  `--text-dim`.
- If `status != null`:
  - Strategy name (large), symbol chip
  - Status badge:
    - `Running` → `--text-green`
    - `Paused` → `#c8a84b` (yellow)
    - `Failed` → `#c85a54` (red)
    - `Stopped` → `--text-dim`
    - `Starting` → `--text-dim`
  - Start time as `"Started HH:MM:SS"`
  - Realized loss as `"Session loss: ₹{n.toFixed(2)}"`:
    - Red (`#c85a54`) if `realizedLoss > 0`
    - `--text-dim` if `realizedLoss === 0`
    - (No negative case — the value is a magnitude.)
  - **Stale badge**: if `status.lossTrackingStale === true`, show a
    prominent red banner: "⚠ Loss tracking is stale — broker connectivity
    issue. Session may auto-pause."
  - **Fail reason**: if status is `Failed`, show `status.failReason` in
    red below the badges.

**Section 2 — Positions Card** (shown when status is `Running` or
`Paused`):
- Header: "Open Positions" with count badge from
  `status.positionCount`.
- Currently `getLiveStatus` returns only the count, not the position
  list. If you need the list, add a separate `getLivePositions` command
  in the backend — but for Phase 7 the count is enough. Show a table
  only if a `getLivePositions` command exists; otherwise render just
  the count and a note "Detailed positions view coming in Phase 8".
- Ask the user before adding a new backend command.

**Section 3 — Controls Row** (shown when a session exists):
- Three ghost `Button` components:
  - **Pause**: `pauseLiveStrategy()`. Disabled unless
    `status === 'Running'`.
  - **Resume**: `resumeLiveStrategy()`. Disabled unless
    `status === 'Paused'`.
  - **Stop**: red styling (`color: #c85a54; border-color: #c85a54`).
    `stopLiveStrategy()`. Disabled while `status === 'Starting'`. On
    success, if `openPositionsWarning` is non-null, show as a toast/
    alert. The backend also emits a
    `live_session_stopped_with_positions` event which the toast layer
    (D below) already handles — pick one path (event listener) and
    remove the return-value path from this button to avoid double
    toasts. Preferred: rely on the event.

Under Pause, add a subtitle in `--text-dim`: "Pausing stops new entries
only. Existing stops and risk rules continue to run." This matches the
fixed pause semantics (Fix 5 in the fix pack).

**Section 4 — Trade Log Card**:
- Header: "Trade Log" with count badge.
- Columns: Time | Strategy | Symbol | Side | Qty | Price | Status |
  Order ID | Notes
- Add a **Status** column (new vs original spec): shows
  `orderStatus` — "TRADED" in green, "TRANSIT"/"PENDING" in yellow,
  "REJECTED"/"CANCELLED"/"EXPIRED" in red.
- **Side**: "BUY" green, "SELL" red.
- **Price**: if the row's `orderStatus !== 'TRADED'` (i.e. not filled),
  display as `—` in `--text-dim` instead of `0.00`.
- **Notes**: dim if empty, red if `"stop_loss"` or `"risk_breach"`,
  yellow if `"take_profit"`.
- Fetch via `getTradeLog()` on mount. "↻ Refresh" ghost button.
- Empty state: "No live trades recorded yet." in `--text-dim`.
- Scrollable, `max-height: 280px`.

### C. Confirmation Modal — `src/components/LiveConfirmModal/LiveConfirmModal.tsx`

Two-step flow with an ack sub-step in between if needed.

Internal state:
```typescript
type ModalStep = "preflight" | "ack" | "confirm" | "error";

const [step, setStep] = useState<ModalStep>("preflight");
const [token, setToken] = useState<string | null>(null);
const [error, setError] = useState<string | null>(null);
const [countdown, setCountdown] = useState(3);
```

**Step 1 — Pre-flight**:
- Title: "Start Live Trading"
- Body: bullet list of checks: "Market hours check", "Broker
  connectivity check", "Symbol map check", "Segment check (NSE equity
  only)", "Risk controls check", "MAX_DAILY_LOSS declared", "Broker
  loss-tracking healthy".
- Warning box: `background: rgba(200, 90, 84, 0.1); border: 1px solid
  #c85a54; border-radius: 6px; padding: 12px;`
  Text: "⚠ Live trading places real orders with real money. Losses may
  exceed your configured limits in fast markets. Stopping the session
  does not auto-close positions or auto-cancel pending orders — you
  must handle those in your broker app. Make sure you have reviewed
  your strategy's backtest."
- Buttons: "Cancel" (ghost), "Run Checks" (primary).
- On "Run Checks":
  ```typescript
  try {
      const res = await requestLiveStart(strategyId);
      setToken(res.token);
      if (res.requiresAck) setStep("ack");
      else { setStep("confirm"); startCountdown(); }
  } catch (e) {
      setError(String(e));
  }
  ```
  Show the error in red under the buttons and stay on `preflight`.

**Step 1.5 — First-live acknowledgment** (`step === "ack"`):
- Title: "First Live Trade Warning"
- Body: "You are about to place your first live order on AlgoMLN. Once
  confirmed, real orders will be sent to Dhan on your behalf. Paper
  trading is always available and recommended for new strategies. Do
  you understand and accept the risks?"
- Buttons: "Cancel", "I Understand — Proceed" (primary).
- On "I Understand":
  ```typescript
  try {
      await acknowledgeLiveTrading();
      // token from Step 1 is still valid — advance
      setStep("confirm");
      startCountdown();
  } catch (e) {
      setError(String(e));
      setStep("preflight");   // ack failed, restart
  }
  ```

**Step 2 — Confirm** (`step === "confirm"`):
- Title: "Confirm Live Start"
- Body: strategy name, symbol, and: "Reminder: stopping the session
  does NOT auto-close open positions or cancel pending orders."
- Countdown: starts at 3, ticks down once per second via `setInterval`
  cleared on unmount and step change. "Confirm — Go Live" button shows
  "Wait (3)…", "Wait (2)…", "Wait (1)…", then "Confirm — Go Live"
  (enabled when countdown reaches 0).
- On confirm click:
  ```typescript
  if (token === null) { setError("no token"); setStep("preflight"); return; }
  try {
      await confirmLiveStart(strategyId, token);
      onSuccess();   // parent closes modal + navigates to Live screen
  } catch (e) {
      setError(String(e));
      setToken(null);
      setStep("preflight");   // token is now consumed; restart
  }
  ```

Cancel from any step calls `onCancel()` (parent closes modal). Clearing
timers on unmount is mandatory.

### D. `src/screens/Strategies/StrategiesScreen.tsx`

Add "Go Live" button to each `StrategyCard` where `mode === 'live'`.
Clicking opens `LiveConfirmModal` with the strategy's id. After the
flow completes (`onSuccess`), close the modal, bump the strategies
refresh key, navigate to the Live screen via a prop callback
`onNavigateToLive`.

### E. `src/components/Sidebar/Sidebar.tsx`

Add fifth nav item between `Plugins` and `Settings`:
```typescript
{ id: 'live', label: 'Live', icon: '◉' }
```

### F. `src/App.tsx`

- Add `'live'` to the `Screen` type.
- Add `LiveScreen` to the render switch.
- Add `liveConfirmStrategyId: string | null` to state.
- Pass `onGoLive={(id) => setLiveConfirmStrategyId(id)}` and
  `onNavigateToLive={() => setScreen('live')}` into `StrategiesScreen`.
- Render `<LiveConfirmModal />` when `liveConfirmStrategyId !== null`.

### G. Event listeners — `src/App.tsx` or a dedicated `useLiveEvents` hook

Register global Tauri event listeners:

```typescript
import { listen } from "@tauri-apps/api/event";

useEffect(() => {
    if (!isTauri()) return;
    const unlistenFail = listen<LiveSessionFailedPayload>(
        "live_session_failed",
        (event) => {
            showToast({
                kind: "error",
                title: "Live session failed",
                body: `${event.payload.reason}. Open positions may need manual attention.`,
                sticky: true,   // do not auto-dismiss
            });
        }
    );
    const unlistenStop = listen<LiveSessionStoppedPayload>(
        "live_session_stopped_with_positions",
        (event) => {
            showToast({
                kind: "warning",
                title: "Session stopped",
                body: event.payload.warning,
                sticky: true,
            });
        }
    );
    return () => {
        unlistenFail.then(f => f());
        unlistenStop.then(f => f());
    };
}, []);
```

If a toast system does not already exist in the app, ask the user
before adding one. A minimal implementation: a `ToastContext` with an
array of `{ id, kind, title, body, sticky }` rendered as fixed-position
cards in the top-right corner, dismissable.

### H. Browser fallback

`requestLiveStart`, `confirmLiveStart`, `acknowledgeLiveTrading`,
`pauseLiveStrategy`, `resumeLiveStrategy`, `stopLiveStrategy` — throw
"live trading is not available in the browser" when `isTauri() === false`
(already handled in the Prompt 5 wrappers).

`getLiveStatus()` returns `null` in browser. `getTradeLog()` returns `[]`.
The `LiveScreen` and modals must render sensibly in both cases — never
throw during a browser preview. `LiveConfirmModal` should show a clear
"live trading is only available in the desktop app" message with only a
Cancel button when running in-browser.

---

## After coding

Ask the user to run:

```
npm run build
npm run dev
```

Wait for output. If `npm run build` errors, ask for full output.

For `npm run dev`, ask the user to:
- Confirm the sidebar shows the new `Live` item.
- Confirm the Strategies screen shows a "Go Live" button on live-mode
  cards.
- (Browser only) Confirm clicking "Go Live" shows the browser-not-
  supported message.

---

## Ask the user (before finalising)

Two prompts:

1. Section B (Positions Card): the current backend only exposes
   `positionCount`, not the position list. Ask:
   > "The current `getLiveStatus` returns only the position count.
   > Do you want me to add a new `get_live_positions` command that
   > returns the full list for the positions table? Or defer that to
   > Phase 8?"

2. Section G: ask:
   > "The app needs a toast system for the sticky failure/stopped-with-
   > positions notifications. Is there an existing toast component I
   > should use, or should I add a minimal `ToastContext`?"

Do not silently invent either.
