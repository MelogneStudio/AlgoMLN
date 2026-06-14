# Scaling System for AlgoMLN

## The Core Idea

The app has a fixed design size of 1550×757px. Rather than making the layout fluid (which would break the precise Figma-matched positioning), the scaling system works by applying a **CSS transform scale** to the entire app window, then resizing the Tauri window to match. Everything inside stays at its design dimensions — the scale factor just shrinks or grows the whole canvas uniformly.

---

## Scale Factor Calculation

There are two sources of scale:

**1. Fit-to-screen (automatic):** On launch, compute the maximum scale that fits the window inside the available screen space with some breathing room.

```typescript
const DESIGN_WIDTH = 1550;
const DESIGN_HEIGHT = 757;
const SCREEN_PADDING = 40; // px breathing room on each side

function computeFitScale(screenW: number, screenH: number): number {
  const maxW = screenW - SCREEN_PADDING * 2;
  const maxH = screenH - SCREEN_PADDING * 2;
  const scaleX = maxW / DESIGN_WIDTH;
  const scaleY = maxH / DESIGN_HEIGHT;
  return Math.min(scaleX, scaleY, 1.0); // never scale above 1.0 automatically
}
```

**2. User override (Settings slider):** The user can pick a scale from 0.6× to 1.4× in 0.05 steps. This overrides the auto-fit value. Stored in `localStorage` as `algomln_ui_scale`.

The effective scale is: `userScale ?? fitScale`. If the user has never set a preference, auto-fit is used. If they set one, it persists.

---

## How the Transform Works

The `AppWindow` component applies the scale to itself:

```css
/* AppWindow.module.css */
.window {
  width: 1550px;
  height: 757px;
  transform-origin: top left;
  /* scale injected as inline CSS variable */
  transform: scale(var(--ui-scale, 1));
  /* everything else as before */
}
```

```tsx
// AppWindow.tsx
<div
  className={styles.window}
  style={{ '--ui-scale': scale } as React.CSSProperties}
>
```

Then the Tauri window itself is resized to match the scaled output so the OS window border hugs the content:

```typescript
import { getCurrentWindow, LogicalSize } from '@tauri-apps/api/window';

async function applyScale(scale: number) {
  const win = getCurrentWindow();
  await win.setSize(new LogicalSize(
    Math.round(DESIGN_WIDTH * scale),
    Math.round(DESIGN_HEIGHT * scale),
  ));
}
```

Call `applyScale` on startup (after computing fit scale) and whenever the user changes the slider in Settings.

---

## The Settings Slider

In the Settings screen, under a new **Card 4 — Display:**

- Label: "Interface Scale"
- A custom range slider from 0.6 to 1.4, step 0.05
- Displays the current value as `0.85×`, `1.00×`, etc., in Cascadia Code Bold
- A "Reset to Auto" button that clears `localStorage` and reverts to fit-scale
- Live preview: the scale applies immediately as the user drags (debounced 16ms so it's smooth), and the Tauri window resizes in real time

The slider must itself be styled to match the design language — no native browser range input styling. Use a custom component (`ScaleSlider`) built the same way as `OptionSlider` but continuous rather than discrete. A filled track up to the thumb, an empty track after, a draggable pill thumb in `var(--highlight)`.

**Critical math note:** The slider stores and operates on the raw float scale value (e.g. `0.85`). When displaying, multiply by 100 and format as a percentage if preferred, or just show `0.85×`. When passing to CSS, pass the raw float. Do not round it for the CSS — CSS handles sub-pixel transforms fine.

---

## Minimum Scale and the Sidebar Collapse Lock

**Minimum allowed scale: 0.6×**

At 0.6× the effective resolution is `930×454px` — below this the UI becomes unusable and text becomes unreadable at normal OS DPI.

**Sidebar collapse lock threshold: 0.75×**

Below `0.75×`, the sidebar must be **permanently collapsed** — the toggle button is hidden and the sidebar width is locked to `var(--sidebar-collapsed-width)` (84px in design units, which becomes 63px at 0.75×). The reason: the expanded sidebar at 280px design units would eat 36% of the 760px effective width at 0.75×, making the content area too narrow to be usable.

```typescript
const SIDEBAR_FORCE_COLLAPSE_THRESHOLD = 0.75;

// In Sidebar component:
const isForcedCollapsed = scale < SIDEBAR_FORCE_COLLAPSE_THRESHOLD;
const isCollapsed = isForcedCollapsed || userCollapsed;

// Hide the toggle button entirely when forced:
{!isForcedCollapsed && <SidebarToggleButton />}
```

When the user increases scale above 0.75× again, the sidebar returns to whatever state it was in before (remember `userCollapsed` separately from `isForcedCollapsed`).

---

## Where Scale Lives

Scale is a piece of app-global state, not component state. It lives in `App.tsx` alongside the screen/modal state:

```typescript
const [scale, setScale] = useState<number>(() => {
  const saved = localStorage.getItem('algomln_ui_scale');
  return saved ? parseFloat(saved) : computeFitScaleFromScreen();
});
```

Pass `scale` down as a prop to:
- `AppWindow` (applies the CSS transform)
- `Sidebar` (determines collapse lock)
- `SettingsScreen` (renders the slider and current value)

Do not put scale in a context — the prop tree is shallow enough that drilling is cleaner and avoids hidden re-render bugs.

---

## Auto-Scale on Window Resize

The OS window itself is not resizable by the user (it's a fixed-size Tauri window), so you don't need to handle arbitrary resize events. However, the user might move the app between monitors with different resolutions. Handle `window.screen` change:

```typescript
useEffect(() => {
  if (localStorage.getItem('algomln_ui_scale')) return; // user has a preference, don't override
  
  const handleScreenChange = () => {
    const newFitScale = computeFitScaleFromScreen();
    setScale(newFitScale);
    applyScale(newFitScale);
  };
  
  // screen change isn't a reliable DOM event — poll every 2s only when no user preference is set
  const interval = setInterval(handleScreenChange, 2000);
  return () => clearInterval(interval);
}, []);
```

This is intentionally lightweight. If a user drags the app to a smaller monitor and it clips, the auto-reset button in Settings fixes it in one click.

---

## Windows Display Scaling Interaction

This is the tricky one. Windows has its own DPI scaling (100%, 125%, 150%, 200%). Tauri's `LogicalSize` already accounts for DPI automatically — logical pixels are device-independent. So `new LogicalSize(1550, 757)` always produces a window that looks 1550×757 in CSS pixels regardless of Windows display scaling. **No special handling needed for the Tauri window size.**

However, the CSS `transform: scale()` works in CSS pixels, which are already DPI-normalized. So the scale math is clean: design units × scale factor = CSS pixels, and Tauri's logical size equals CSS pixels. The full chain:

```
Design units (1550×757)
  × UI scale (e.g. 0.85)
  = CSS pixels (1317×643)
  = Tauri logical pixels (1317×643)
  = Physical pixels (varies by DPI, handled by OS/Tauri)
```

The one thing to watch: if the user is on a 200% Windows display and a 1.4× scale, the physical window becomes `1550 × 1.4 × 2 = 4340px` wide — this might clip on a 1920px-wide screen. The `computeFitScale` function should query the actual screen resolution via `window.screen.width` and `window.screen.height`, which in Tauri's webview context reports the **physical** resolution divided by DPI scale (i.e., CSS pixels available). So `window.screen.width` on a 1920px 200%-DPI screen returns `960` — and `computeFitScale(960, 540)` would correctly compute a small scale. This behavior is correct and no workaround is needed.

---

## Layout Philosophy: Why Transform Scale Instead of Fluid Layout

The alternative — making every px value a CSS `calc()` against a scale variable — would require touching every single component and every single numeric value in every module CSS file. It would also mean the positions that were matched pixel-perfectly to Figma would drift due to floating point differences between properties. Transform scale avoids all of this: the Figma positions remain correct at `1.0×`, and the GPU scales the rendered output uniformly. Text rendering at sub-1.0 scale will be slightly softened by antialiasing, but that's the correct tradeoff.

The one limitation of this approach: the CSS `backdrop-filter` on the sidebar may not scale visually with the transform on some GPU drivers. If that occurs, recompute the blur radius as `calc(45px * var(--ui-scale))` on the sidebar specifically — that's the only exception to the "don't touch component values" rule.