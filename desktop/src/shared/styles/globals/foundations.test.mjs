import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";

import tailwindConfig from "../../../../tailwind.config.js";

const themeCss = readFileSync(new URL("./theme.css", import.meta.url), "utf8");
const { borderRadius, colors, fontSize } = tailwindConfig.theme.extend;

test("semantic type roles preserve Buzz's current shared type ramp", () => {
  assert.deepEqual(fontSize["display-page-title"], [
    "1.5rem",
    {
      lineHeight: "2rem",
      fontWeight: "600",
      letterSpacing: "-0.025em",
    },
  ]);
  assert.deepEqual(fontSize["display-section-title"], [
    "1.125rem",
    {
      lineHeight: "1.75rem",
      fontWeight: "600",
      letterSpacing: "-0.025em",
    },
  ]);
  assert.deepEqual(fontSize["body-medium"], [
    "1rem",
    { lineHeight: "1.5rem", fontWeight: "400" },
  ]);
  assert.deepEqual(fontSize["label-medium"], [
    "1rem",
    { lineHeight: "1.5rem", fontWeight: "600" },
  ]);
  assert.deepEqual(fontSize["body-small"], [
    "0.875rem",
    { lineHeight: "1.25rem", fontWeight: "400" },
  ]);
  assert.deepEqual(fontSize.caption, [
    "0.75rem",
    { lineHeight: "1rem", fontWeight: "400" },
  ]);
});

test("semantic color families expose the BlockUI role vocabulary", () => {
  assert.deepEqual(Object.keys(colors.content), [
    "standard",
    "subtle",
    "muted",
    "disabled",
    "inverse",
  ]);
  assert.deepEqual(Object.keys(colors.icon), [
    "standard",
    "subtle",
    "muted",
    "disabled",
    "inverse",
  ]);
  assert.deepEqual(Object.keys(colors.surface), [
    "app",
    "subtle",
    "standard",
    "prominent",
    "inverse",
    "card",
    "popover",
  ]);
  assert.deepEqual(Object.keys(colors.border).slice(1), [
    "subtle",
    "standard",
    "prominent",
    "focus",
  ]);
  assert.deepEqual(Object.keys(colors.status).slice(3), [
    "error",
    "warning",
    "success",
    "info",
    "notification",
  ]);
});

test("semantic color aliases preserve the current theme mappings", () => {
  const expectedAliases = {
    "content-standard": "hsl(var(--foreground))",
    "content-subtle": "hsl(var(--muted-foreground))",
    "content-muted": "hsl(var(--muted-foreground))",
    "content-disabled": "hsl(var(--muted-foreground))",
    "surface-app": "hsl(var(--background))",
    "surface-card": "hsl(var(--card))",
    "surface-popover": "hsl(var(--popover))",
    "border-subtle": "hsl(var(--border))",
    "border-standard": "hsl(var(--border))",
    "border-focus": "hsl(var(--ring))",
    "status-error": "hsl(var(--destructive))",
    "status-warning": "var(--ui-warning)",
    "status-success": "var(--status-added)",
  };

  for (const [alias, currentValue] of Object.entries(expectedAliases)) {
    assert.match(
      themeCss,
      new RegExp(`--${alias}:\\s*${currentValue.replace(/[()*-]/g, "\\$&")};`),
      `${alias} should continue to resolve to ${currentValue}`,
    );
  }
});

test("semantic shape roles preserve Buzz's current radius steps", () => {
  assert.deepEqual(
    {
      control: borderRadius["shape-control"],
      notice: borderRadius["shape-notice"],
      menu: borderRadius["shape-menu"],
      container: borderRadius["shape-container"],
      overlay: borderRadius["shape-overlay"],
      sheet: borderRadius["shape-sheet"],
      pill: borderRadius["shape-pill"],
      circular: borderRadius["shape-circular"],
    },
    {
      control: "var(--shape-control)",
      notice: "var(--shape-notice)",
      menu: "var(--shape-menu)",
      container: "var(--shape-container)",
      overlay: "var(--shape-overlay)",
      sheet: "var(--shape-sheet)",
      pill: "var(--shape-pill)",
      circular: "var(--shape-circular)",
    },
  );

  for (const [role, value] of [
    ["control", "calc(var(--radius) - 2px)"],
    ["notice", "var(--radius)"],
    ["menu", "1rem"],
    ["container", "1.5rem"],
    ["overlay", "2rem"],
    ["sheet", "2.5rem"],
    ["pill", "9999px"],
    ["circular", "9999px"],
  ]) {
    assert.match(
      themeCss,
      new RegExp(`--shape-${role}:\\s*${value.replace(/[()*-]/g, "\\$&")};`),
    );
  }
});
