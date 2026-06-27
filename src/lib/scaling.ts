import { getCurrentWindow, LogicalSize } from '@tauri-apps/api/window';

export const DESIGN_WIDTH = 1550;
export const DESIGN_HEIGHT = 757;
export const SCREEN_PADDING = 40;
export const SIDEBAR_FORCE_COLLAPSE_THRESHOLD = 0.75;
export const CAPITAL_STORAGE_KEY = 'algomln_default_capital';

/**
 * Maximum uniform scale that fits the design canvas inside `screenW x screenH`
 * with `SCREEN_PADDING` breathing room on each side. Capped at 1.0 — we never
 * upscale the design canvas.
 *
 * This is the *only* source of scale. It is computed once per launch from the
 * current screen and never changes afterwards (no user override).
 */
export function computeFitScale(screenW: number, screenH: number): number {
  const maxW = Math.max(0, screenW - SCREEN_PADDING * 2);
  const maxH = Math.max(0, screenH - SCREEN_PADDING * 2);
  if (maxW <= 0 || maxH <= 0) return 1;
  const scaleX = maxW / DESIGN_WIDTH;
  const scaleY = maxH / DESIGN_HEIGHT;
  return Math.min(scaleX, scaleY, 1.0);
}

/**
 * Read the available screen size. In Tauri webview this returns CSS pixels
 * (DPI-normalized). In a plain browser we fall back to window.innerWidth/Height.
 */
export function getScreenSize(): { w: number; h: number } {
  if (typeof window === 'undefined') return { w: 0, h: 0 };
  const w = window.screen?.width ?? window.innerWidth;
  const h = window.screen?.height ?? window.innerHeight;
  return { w, h };
}

/**
 * Size the OS window to the scaled canvas and center it on the desktop.
 * No-op when not running in Tauri (e.g. `npm run dev`).
 *
 * The window is created with native decorations off (see tauri.conf.json), so
 * outer size == inner size and there is no chrome overhead to compensate for.
 * The CSS transform shrinks the canvas to `DESIGN * scale`; this makes the OS
 * window hug exactly that, so there is no leftover transparent area around it.
 */
export async function applyScale(scale: number): Promise<void> {
  if (typeof window === 'undefined') return;
  if (!('__TAURI_INTERNALS__' in window)) return;
  try {
    const win = getCurrentWindow();
    const targetW = Math.round(DESIGN_WIDTH * scale);
    const targetH = Math.round(DESIGN_HEIGHT * scale);
    await win.setSize(new LogicalSize(targetW, targetH));
    await win.center();
  } catch (err) {
    // Silently ignore — setting size on certain backends can fail.
    console.warn('applyScale failed:', err);
  }
}

export function loadSavedCapital(fallback = 100_000): number {
  if (typeof window === 'undefined') return fallback;
  const raw = window.localStorage.getItem(CAPITAL_STORAGE_KEY);
  if (!raw) return fallback;
  const parsed = parseFloat(raw);
  if (!Number.isFinite(parsed) || parsed <= 0) return fallback;
  return parsed;
}

export function saveCapital(value: number): void {
  if (typeof window === 'undefined') return;
  window.localStorage.setItem(CAPITAL_STORAGE_KEY, String(value));
}
