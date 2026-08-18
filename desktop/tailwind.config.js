/** @type {import('tailwindcss').Config} */
export default {
  theme: {
    extend: {
      // Consolidated type ramp: 6 prose sizes + 1 mono. Every text class in
      // the app compiles onto one of these steps:
      //
      //   Meta 12 (xs) · Body 14 (sm) · Title 18 (lg) · Section 24 (2xl) ·
      //   Page 30 (3xl) · Display 40 (title) · Key 36 (nsec-key, mono)
      //
      // Legacy class names are remapped here instead of rewritten at every
      // call site: the sub-12 meta band (3xs 8px, badge 10px, 2xs 11px) folds
      // up to Meta 12, `text-base` (16px) folds down to Body 14, `text-xl`
      // (20px) folds down to Title 18, and `text-4xl` (36px) / `text-5xl`
      // (48px) fold to Display 40. Defined in rem so Cmd +/- zoom — which
      // scales the root <html> font-size — keeps scaling them. Do NOT
      // reintroduce arbitrary `text-[…rem]` / `text-[…px]` literals; the
      // px-text guard rejects them.
      fontSize: {
        // Meta 12 — timestamps, count badges, tracking labels, tiny glyphs.
        // Aliases of the stock `text-xs` step.
        "2xs": "0.75rem",
        "3xs": "0.75rem",
        badge: "0.75rem",
        // Body 14 — folds the old 16px body/description step down one clean
        // step below its 18px header. Matches stock `text-sm` exactly.
        base: ["0.875rem", { lineHeight: "1.25rem" }],
        // Title 18 — folds the old 20px step onto stock `text-lg`.
        xl: ["1.125rem", { lineHeight: "1.75rem" }],
        // Display 40 — the single large display step (was 36/40/48).
        "4xl": ["2.5rem", { lineHeight: "1.15" }],
        "5xl": ["2.5rem", { lineHeight: "1.15" }],
        // 40px — onboarding page titles (tightened tracking for large display type)
        title: ["2.5rem", { lineHeight: "1.15", letterSpacing: "-0.02em" }],
        // 36px — the backup-step private key, shown large in monospace
        "nsec-key": ["2.25rem", { lineHeight: "1.3" }],
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
        border: "hsl(var(--border))",
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
