/**
 * Light / dark / system theme, persisted in localStorage (best-effort).
 *
 * `system` follows `prefers-color-scheme` live. public/theme-init.js applies
 * the same logic before first paint to avoid a flash.
 */

import { create } from 'zustand';
import { readStorage, STORAGE_KEYS, writeStorage } from '@/lib/storage';

export type Theme = 'light' | 'dark' | 'system';
export type ResolvedTheme = 'light' | 'dark';

function systemPrefersDark(): boolean {
  try {
    return window.matchMedia('(prefers-color-scheme: dark)').matches;
  } catch {
    return false;
  }
}

export function resolveTheme(theme: Theme): ResolvedTheme {
  if (theme === 'system') return systemPrefersDark() ? 'dark' : 'light';
  return theme;
}

function applyTheme(resolved: ResolvedTheme): void {
  const root = document.documentElement;
  root.classList.toggle('dark', resolved === 'dark');
  root.style.colorScheme = resolved;
}

function readSavedTheme(): Theme {
  const saved = readStorage('local', STORAGE_KEYS.theme);
  return saved === 'light' || saved === 'dark' || saved === 'system' ? saved : 'system';
}

interface ThemeState {
  theme: Theme;
  resolved: ResolvedTheme;
  setTheme: (theme: Theme) => void;
}

const initialTheme: Theme = typeof window === 'undefined' ? 'system' : readSavedTheme();

export const useThemeStore = create<ThemeState>((set) => ({
  theme: initialTheme,
  resolved: typeof window === 'undefined' ? 'light' : resolveTheme(initialTheme),
  setTheme: (theme) => {
    writeStorage('local', STORAGE_KEYS.theme, theme);
    const resolved = resolveTheme(theme);
    applyTheme(resolved);
    set({ theme, resolved });
  },
}));

if (typeof window !== 'undefined') {
  applyTheme(useThemeStore.getState().resolved);
  try {
    window.matchMedia('(prefers-color-scheme: dark)').addEventListener('change', () => {
      const { theme } = useThemeStore.getState();
      if (theme !== 'system') return;
      const resolved = resolveTheme('system');
      applyTheme(resolved);
      useThemeStore.setState({ resolved });
    });
  } catch {
    // matchMedia unavailable; the theme simply won't follow the OS live.
  }
  // Keep tabs in sync when the theme changes elsewhere.
  window.addEventListener('storage', (event) => {
    if (event.key !== STORAGE_KEYS.theme) return;
    const theme = readSavedTheme();
    const resolved = resolveTheme(theme);
    applyTheme(resolved);
    useThemeStore.setState({ theme, resolved });
  });
}
