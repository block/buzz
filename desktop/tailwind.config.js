/** @type {import('tailwindcss').Config} */
export default {
  theme: {
    extend: {
      // Sub-`text-xs` ramp for meta text (timestamps, count badges, tracking
      // labels) and tiny glyphs. Defined in rem so Cmd +/- zoom — which scales
      // the root <html> font-size — keeps scaling them. Do NOT reintroduce
      // arbitrary `text-[…rem]` / `text-[…px]` literals; the px-text guard
      // rejects them. Stock scale picks up from here: xs (12px), sm (14px)…
      fontSize: {
        "2xs": "0.6875rem", // 11px — meta-text workhorse (timestamps, badges)
        "3xs": "0.5rem", // 8px — tiny glyphs / micro labels
        badge: "0.625rem", // 10px — compact status badges
        // 40px — onboarding page titles (tightened tracking for large display type)
        title: ["2.5rem", { lineHeight: "1.15", letterSpacing: "-0.02em" }],
        // 36px — the backup-step private key, shown large in monospace
        "nsec-key": ["2.25rem", { lineHeight: "1.3" }],

        // BlockUI-aligned semantic type roles. These intentionally resolve to
        // Buzz's current shared-header and body styles; adopting BlockUI's
        // target scale is a separate, visual migration. Keeping the role name
        // stable lets that happen without another feature-level rename.
        "display-page-title": [
          "1.5rem",
          {
            lineHeight: "2rem",
            fontWeight: "600",
            letterSpacing: "-0.025em",
          },
        ],
        "display-section-title": [
          "1.125rem",
          {
            lineHeight: "1.75rem",
            fontWeight: "600",
            letterSpacing: "-0.025em",
          },
        ],
        "body-medium": ["1rem", { lineHeight: "1.5rem", fontWeight: "400" }],
        "label-medium": ["1rem", { lineHeight: "1.5rem", fontWeight: "600" }],
        "body-small": [
          "0.875rem",
          { lineHeight: "1.25rem", fontWeight: "400" },
        ],
        "label-small": [
          "0.875rem",
          { lineHeight: "1.25rem", fontWeight: "600" },
        ],
        caption: ["0.75rem", { lineHeight: "1rem", fontWeight: "400" }],
      },
      boxShadow: {
        "content-edge": "-1px -1px 0 0 hsl(var(--sidebar-border) / 0.45)",
        // Edge + elevation for a surface anchored to the right of the content
        // area, whose only exposed edge faces left. Tailwind's stock shadows are
        // all y-offset, so they cast almost nothing sideways — `shadow-xl` on a
        // left-facing edge is nearly invisible. Both layers run -x so they wrap
        // the surface's rounded left corners: the hairline draws the boundary
        // (and carries dark mode, where a black shadow reads as nothing), the
        // soft layer carries the lift. A left-only `border` can't do this job —
        // it tapers out at each corner instead of turning it.
        "panel-left":
          "-1px 0 0 0 hsl(var(--border) / 0.8), -16px 0 32px -12px rgb(0 0 0 / 0.18)",
      },
      borderRadius: {
        lg: "var(--radius)",
        md: "calc(var(--radius) - 2px)",
        sm: "calc(var(--radius) - 4px)",
        // Semantic aliases resolve to radius steps Buzz already uses. The
        // aliases are intentionally non-visual until primitives opt into them.
        "shape-control": "var(--shape-control)",
        "shape-notice": "var(--shape-notice)",
        "shape-menu": "var(--shape-menu)",
        "shape-container": "var(--shape-container)",
        "shape-overlay": "var(--shape-overlay)",
        "shape-sheet": "var(--shape-sheet)",
        "shape-pill": "var(--shape-pill)",
        "shape-circular": "var(--shape-circular)",
      },
      spacing: {
        4.5: "1.125rem",
      },
      fontFamily: {
        sans: [
          '"Inter Variable"',
          "Inter",
          '"Avenir Next"',
          '"Segoe UI"',
          "sans-serif",
        ],
      },
      colors: {
        background: "hsl(var(--background))",
        foreground: "hsl(var(--foreground))",
        content: {
          standard: "var(--content-standard)",
          subtle: "var(--content-subtle)",
          muted: "var(--content-muted)",
          disabled: "var(--content-disabled)",
          inverse: "var(--content-inverse)",
        },
        icon: {
          standard: "var(--icon-standard)",
          subtle: "var(--icon-subtle)",
          muted: "var(--icon-muted)",
          disabled: "var(--icon-disabled)",
          inverse: "var(--icon-inverse)",
        },
        surface: {
          app: "var(--surface-app)",
          subtle: "var(--surface-subtle)",
          standard: "var(--surface-standard)",
          prominent: "var(--surface-prominent)",
          inverse: "var(--surface-inverse)",
          card: "var(--surface-card)",
          popover: "var(--surface-popover)",
        },
        card: {
          DEFAULT: "hsl(var(--card))",
          foreground: "hsl(var(--card-foreground))",
        },
        popover: {
          DEFAULT: "hsl(var(--popover))",
          foreground: "hsl(var(--popover-foreground))",
        },
        primary: {
          DEFAULT: "hsl(var(--primary))",
          foreground: "hsl(var(--primary-foreground))",
        },
        secondary: {
          DEFAULT: "hsl(var(--secondary))",
          foreground: "hsl(var(--secondary-foreground))",
        },
        muted: {
          DEFAULT: "hsl(var(--muted))",
          foreground: "hsl(var(--muted-foreground))",
        },
        accent: {
          DEFAULT: "hsl(var(--accent))",
          foreground: "hsl(var(--accent-foreground))",
        },
        destructive: {
          DEFAULT: "hsl(var(--destructive))",
          foreground: "hsl(var(--destructive-foreground))",
        },
        border: {
          DEFAULT: "hsl(var(--border))",
          subtle: "var(--border-subtle)",
          standard: "var(--border-standard)",
          prominent: "var(--border-prominent)",
          focus: "var(--border-focus)",
        },
        input: "hsl(var(--input))",
        ring: "hsl(var(--ring))",
        sidebar: {
          DEFAULT: "hsl(var(--sidebar-background))",
          foreground: "hsl(var(--sidebar-foreground))",
          primary: "hsl(var(--sidebar-primary))",
          "primary-foreground": "hsl(var(--sidebar-primary-foreground))",
          active: "hsl(var(--sidebar-active))",
          "active-foreground": "hsl(var(--sidebar-active-foreground))",
          accent: "hsl(var(--sidebar-accent))",
          "accent-foreground": "hsl(var(--sidebar-accent-foreground))",
          border: "hsl(var(--sidebar-border))",
          ring: "hsl(var(--sidebar-ring))",
        },
        status: {
          added: "var(--status-added)",
          deleted: "var(--status-deleted)",
          modified: "var(--status-modified)",
          error: "var(--status-error)",
          warning: "var(--status-warning)",
          success: "var(--status-success)",
          info: "var(--status-info)",
          notification: "var(--status-notification)",
        },
        warning: {
          DEFAULT: "var(--ui-warning)",
          bg: "var(--ui-warning-bg)",
        },
      },
    },
  },
  plugins: [],
};
