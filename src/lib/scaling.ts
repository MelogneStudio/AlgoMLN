import { getCurrentWindow, LogicalSize } from '@tauri-apps/api/window';

export const DESIGN_WIDTH = 1550;
export const DESIGN_HEIGHT = 757;
export const SCREEN_PADDING = 40;
export const MIN_SCALE = 0.6;
export const MAX_SCALE = 1.4;
export const SCALE_STEP = 0.05;
export const SIDEBAR_FORCE_COLLAPSE_THRESHOLD = 0.75;
export const SCALE_STORAGE_KEY = 'algomln_ui_scale';
export const CAPITAL_STORAGE_KEY = 'algomln_default_capital';

/**
 * Maximum uniform scale that fits the design canvas inside `screenW x screenH`
 * with `SCREEN_PADDING` breathing room on each side. Capped at 1.0 — we never
 * upscale the design on first launch; the user can do that in Settings.
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
 * Apply the CSS scale transform by resizing the OS window to match the
 * scaled canvas. No-op when not running in Tauri (e.g. `npm run dev`).
 */
export async function applyScale(scale: number): Promise<void> {
  if (typeof window === 'undefined') return;
  if (!('__TAURI_INTERNALS__' in window)) return;
  try {
    const win = getCurrentWindow();
    await win.setSize(
      new LogicalSize(
        Math.round(DESIGN_WIDTH * scale),
        Math.round(DESIGN_HEIGHT * scale)
      )
    );
  } catch (err) {
    // Silently ignore — setting size on certain Linux backends can fail.
    console.warn('applyScale failed:', err);
  }
}

export function clampScale(scale: number): number {
  return Math.min(MAX_SCALE, Math.max(MIN_SCALE, scale));
}

export function roundToStep(scale: number): number {
  return Math.round(scale / SCALE_STEP) * SCALE_STEP;
}

export function loadSavedScale(): number | null {
  if (typeof window === 'undefined') return null;
  const raw = window.localStorage.getItem(SCALE_STORAGE_KEY);
  if (!raw) return null;
  const parsed = parseFloat(raw);
  if (!Number.isFinite(parsed)) return null;
  return clampScale(parsed);
}

export function saveScale(scale: number): void {
  if (typeof window === 'undefined') return;
  window.localStorage.setItem(SCALE_STORAGE_KEY, String(clampScale(scale)));
}

export function clearSavedScale(): void {
  if (typeof window === 'undefined') return;
  window.localStorage.removeItem(SCALE_STORAGE_KEY);
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
