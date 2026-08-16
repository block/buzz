/**
 * Brand-identity variants (exploration branch — "give Buzz its own face").
 *
 * A variant is a thin layer ON TOP of the theme engine: `applyTheme` /
 * `applyAccentColor` run first, then `applyIdentityVariant` re-asserts the
 * variant's signature vars as inline overrides on `:root` and stamps
 * `data-identity` on `<html>` so the companion CSS (theme.css, "Identity
 * variants" section) can restyle agent rows, type, and geometry.
 *
 * Hooked at the exit points of `applyAccentColor` (ThemeProvider) — every
 * theme or accent application funnels through there, so variant vars always
 * win and never go stale across light/dark flips.
 *
 * Directions:
 *   stock — untouched app (default; no attribute, no overrides)
 *   honey — brand-forward: signature amber accent (the "buzz"/bee identity),
 *           teal agent signature
 *   dual  — human/agent duality: indigo for humans, strong cyan treatment
 *           for agents so a channel reads at a glance
 *   mono  — dev-tool identity: green accent, violet agents, sharp corners,
 *           JetBrains Mono on names/labels
 *
 * Known limitation (fine for exploration): variants are seeded per page load
 * (screenshot harness / localStorage); switching variant mid-session without
 * reload leaves the previous variant's --radius/--primary overrides in place.
 */

export const IDENTITY_STORAGE_KEY = "buzz-identity-variant";

export const IDENTITY_VARIANTS = ["stock", "honey", "dual", "mono"] as const;
export type IdentityVariant = (typeof IDENTITY_VARIANTS)[number];

/** HSL channel triplets (shadcn token format), except --radius (length). */
type VariantVars = {
  light: Record<string, string>;
  dark: Record<string, string>;
};

const VARIANT_VARS: Record<Exclude<IdentityVariant, "stock">, VariantVars> = {
  honey: {
    light: {
      "--primary": "38 92% 44%",
      "--primary-foreground": "0 0% 100%",
      "--sidebar-primary": "38 92% 44%",
      "--sidebar-primary-foreground": "0 0% 100%",
      "--sidebar-active": "38 92% 44%",
      "--sidebar-active-foreground": "0 0% 100%",
      "--buzz-selected-accent": "38 92% 44%",
      "--agent-accent": "174 84% 32%",
    },
    dark: {
      "--primary": "43 96% 56%",
      "--primary-foreground": "24 10% 10%",
      "--sidebar-primary": "43 96% 56%",
      "--sidebar-primary-foreground": "24 10% 10%",
      "--sidebar-active": "43 96% 56%",
      "--sidebar-active-foreground": "24 10% 10%",
      "--buzz-selected-accent": "43 96% 56%",
      "--agent-accent": "174 72% 45%",
    },
  },
  dual: {
    light: {
      "--primary": "239 84% 60%",
      "--primary-foreground": "0 0% 100%",
      "--sidebar-primary": "239 84% 60%",
      "--sidebar-primary-foreground": "0 0% 100%",
      "--sidebar-active": "239 84% 60%",
      "--sidebar-active-foreground": "0 0% 100%",
      "--buzz-selected-accent": "239 84% 60%",
      "--agent-accent": "192 91% 36%",
    },
    dark: {
      "--primary": "239 84% 67%",
      "--primary-foreground": "0 0% 100%",
      "--sidebar-primary": "239 84% 67%",
      "--sidebar-primary-foreground": "0 0% 100%",
      "--sidebar-active": "239 84% 67%",
      "--sidebar-active-foreground": "0 0% 100%",
      "--buzz-selected-accent": "239 84% 67%",
      "--agent-accent": "187 85% 53%",
    },
  },
  mono: {
    light: {
      "--primary": "142 76% 36%",
      "--primary-foreground": "0 0% 100%",
      "--sidebar-primary": "142 76% 36%",
      "--sidebar-primary-foreground": "0 0% 100%",
      "--sidebar-active": "142 76% 36%",
      "--sidebar-active-foreground": "0 0% 100%",
      "--buzz-selected-accent": "142 76% 36%",
      "--agent-accent": "262 83% 58%",
      "--radius": "0.125rem",
    },
    dark: {
      "--primary": "142 69% 45%",
      "--primary-foreground": "24 10% 10%",
      "--sidebar-primary": "142 69% 45%",
      "--sidebar-primary-foreground": "24 10% 10%",
      "--sidebar-active": "142 69% 45%",
      "--sidebar-active-foreground": "24 10% 10%",
      "--buzz-selected-accent": "142 69% 45%",
      "--agent-accent": "262 83% 70%",
      "--radius": "0.125rem",
    },
  },
};

/** Vars only ever set by variants — safe to remove when stock is active. */
const VARIANT_ONLY_VARS = ["--agent-accent"];

export function readIdentityVariant(): IdentityVariant {
  try {
    const stored = window.localStorage.getItem(IDENTITY_STORAGE_KEY);
    if (stored && (IDENTITY_VARIANTS as readonly string[]).includes(stored)) {
      return stored as IdentityVariant;
    }
  } catch {
    // Storage unavailable — fall through to stock.
  }
  return "stock";
}

export function applyIdentityVariant(variant: IdentityVariant): void {
  const root = document.documentElement;
  if (variant === "stock") {
    root.removeAttribute("data-identity");
    for (const key of VARIANT_ONLY_VARS) root.style.removeProperty(key);
    return;
  }
  root.setAttribute("data-identity", variant);
  const isDark = root.classList.contains("dark");
  const vars = VARIANT_VARS[variant][isDark ? "dark" : "light"];
  for (const [key, value] of Object.entries(vars)) {
    root.style.setProperty(key, value);
  }
}
