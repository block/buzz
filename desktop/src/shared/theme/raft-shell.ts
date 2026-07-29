/**
 * BC home_portal "Raft" shell tokens for the default Buzz themes.
 *
 * Syntax highlighting still loads via Shiki (github-light/dark), but the app
 * chrome must use these values — ThemeProvider merges them after
 * createThemeVars so inline styles do not wipe the Raft palette.
 *
 * Values mirror desktop/src/shared/styles/globals/theme.css (:root / .dark).
 */

export type RaftShellVars = Record<string, string>;

/** Amber primary used by Raft (approx #fbbf24). */
export const RAFT_ACCENT_HEX = "#fbbf24";

const RAFT_LIGHT: RaftShellVars = {
  "--radius": "0rem",
  "--background": "30 100% 97%",
  "--foreground": "24 10% 10%",
  "--card": "0 0% 100%",
  "--card-foreground": "24 10% 10%",
  "--popover": "0 0% 100%",
  "--popover-foreground": "24 10% 10%",
  "--primary": "43 96% 56%",
  "--primary-foreground": "24 10% 10%",
  "--secondary": "30 25% 92%",
  "--secondary-foreground": "24 10% 10%",
  "--muted": "30 20% 90%",
  "--muted-foreground": "30 6% 32%",
  "--accent": "43 96% 90%",
  "--accent-foreground": "24 10% 10%",
  "--destructive": "347 77% 50%",
  "--destructive-foreground": "0 0% 100%",
  "--border": "24 10% 10%",
  "--input": "24 10% 10%",
  "--ring": "43 96% 56%",
  /* Charcoal rail + near-white labels (home_portal --side / --side-text) */
  "--sidebar": "24 10% 16%",
  "--sidebar-background": "24 10% 16%",
  "--sidebar-foreground": "40 33% 98%",
  "--sidebar-primary": "43 96% 56%",
  "--sidebar-primary-foreground": "24 10% 10%",
  "--sidebar-active": "43 96% 56%",
  "--sidebar-active-foreground": "24 10% 10%",
  "--sidebar-accent": "24 8% 22%",
  "--sidebar-accent-foreground": "40 33% 98%",
  "--sidebar-border": "24 6% 28%",
  "--sidebar-ring": "43 96% 56%",
};

const RAFT_DARK: RaftShellVars = {
  "--radius": "0rem",
  "--background": "24 10% 10%",
  "--foreground": "60 9% 98%",
  "--card": "24 10% 15%",
  "--card-foreground": "60 9% 98%",
  "--popover": "24 10% 15%",
  "--popover-foreground": "60 9% 98%",
  "--primary": "43 96% 56%",
  "--primary-foreground": "24 10% 10%",
  "--secondary": "24 6% 20%",
  "--secondary-foreground": "60 9% 98%",
  "--muted": "24 6% 20%",
  "--muted-foreground": "30 6% 65%",
  "--accent": "24 6% 22%",
  "--accent-foreground": "60 9% 98%",
  "--destructive": "0 72% 55%",
  "--destructive-foreground": "0 0% 100%",
  "--border": "24 6% 28%",
  "--input": "24 6% 28%",
  "--ring": "43 96% 56%",
  "--sidebar": "24 10% 12%",
  "--sidebar-background": "24 10% 12%",
  "--sidebar-foreground": "60 9% 98%",
  "--sidebar-primary": "43 96% 56%",
  "--sidebar-primary-foreground": "24 10% 10%",
  "--sidebar-active": "43 96% 56%",
  "--sidebar-active-foreground": "24 10% 10%",
  "--sidebar-accent": "24 8% 18%",
  "--sidebar-accent-foreground": "60 9% 98%",
  "--sidebar-border": "24 6% 22%",
  "--sidebar-ring": "43 96% 56%",
};

export function getRaftShellVars(isDark: boolean): RaftShellVars {
  return isDark ? { ...RAFT_DARK } : { ...RAFT_LIGHT };
}

/** Expected computed-style samples for QA / unit checks (HSL channel triples). */
export const RAFT_LIGHT_PRIMARY = RAFT_LIGHT["--primary"];
export const RAFT_LIGHT_BACKGROUND = RAFT_LIGHT["--background"];
export const RAFT_LIGHT_SIDEBAR = RAFT_LIGHT["--sidebar-background"];
