import { readFileSync } from "node:fs";

import { describe, expect, it } from "vitest";

import { PALETTE, RAMPS, ROLE_GROUPS } from "./registry";

/**
 * `tokens.css` as text.
 *
 * Read with `node:fs` rather than Vite's `import.meta.glob(..., "?raw")`, which
 * the sibling component-registry test uses for `.tsx` sources. On a `.css` file
 * that returns an **empty string** — Vite hands it to the CSS pipeline and the
 * raw query yields nothing, with no error. Every assertion below would have
 * passed vacuously against empty text, which is the exact failure mode this file
 * exists to catch, so the length is asserted before anything else.
 */
const TOKENS = readFileSync(
  new URL("../styles/tokens.css", import.meta.url),
  "utf8",
);

/** Declarations outside `@theme`: the role and palette layers. */
const DECLARATIONS = TOKENS.slice(0, TOKENS.indexOf("@theme inline {"));
/** Inside `@theme`: what Tailwind turns into utilities. */
const THEME = TOKENS.slice(TOKENS.indexOf("@theme inline {"));

const declares = (variable: string) =>
  new RegExp(`^\\s*${variable}:`, "m").test(DECLARATIONS);

/**
 * The /design pages are rendered from this registry, so a role described here
 * but absent from the stylesheet is a page confidently documenting something
 * that does not exist — and the reader has no way to tell.
 *
 * This is not hypothetical. `bg-float-glass` was described as "a floating
 * surface that should let content through", rendered a swatch on /design/color,
 * and was never declared anywhere: the swatch painted nothing and the row read
 * as a real token. It survived a census of consumers too, because a token no
 * stylesheet declares also has no consumers to count.
 */
describe("token registry — every documented role exists", () => {
  it("actually read the stylesheet", () => {
    // Guards the guard: see the note on TOKENS. Empty text would make every
    // other assertion here pass while checking nothing.
    expect(TOKENS.length).toBeGreaterThan(1000);
    expect(TOKENS).toContain("@theme inline {");
  });

  it("declares every role the registry describes", () => {
    const missing: string[] = [];
    for (const group of ROLE_GROUPS) {
      for (const role of group.roles) {
        if (!declares(role.variable))
          missing.push(`${group.id}: ${role.variable}`);
      }
    }
    expect(missing).toEqual([]);
  });

  it("declares every palette step and ramp step it lists", () => {
    const missing: string[] = [];
    for (const hue of PALETTE) {
      for (const step of hue.steps) {
        if (!declares(step.variable)) missing.push(step.variable);
      }
    }
    for (const ramp of RAMPS) {
      for (const step of ramp.steps) {
        if (!declares(step.variable)) missing.push(step.variable);
      }
    }
    expect(missing).toEqual([]);
  });
});

/**
 * The namespace rule, bound to the file rather than to a reviewer's memory.
 *
 * `--color-x` is shared by every colour prefix at once — it defines `bg-x`,
 * `text-x`, `border-x` and the rest from one value. So when two roles differ
 * only by which prefix uses them, that namespace silently picks one.
 *
 * It shipped twice. Borders first: `border-primary` resolved to the text colour
 * and drew every hairline at near-black. Then text: `--color-danger` also
 * defined `text-danger`, so error text rendered in red-9, the saturated fill,
 * instead of red-12 — APCA Lc 34 on a dark panel, while `check-contrast` passed
 * because it measured `--text-danger`, a token no class could reach.
 *
 * Both fixes were the same move, so the invariant is one line: a text or border
 * role registers in its own namespace, never the shared one.
 */
describe("token registry — colour namespaces do not collide", () => {
  const registrations = (namespace: string) =>
    [
      ...THEME.matchAll(
        new RegExp(
          `^\\s*--${namespace}-([a-z0-9-]+):\\s*var\\((--[a-z0-9-]+)\\)`,
          "gm",
        ),
      ),
    ].map(([, suffix, role]) => ({ suffix, role }));

  it("registers no text or border role under the shared --color-* namespace", () => {
    const shared = registrations("color").filter(
      ({ role }) => role.startsWith("--text-") || role.startsWith("--border-"),
    );
    expect(
      shared.map(({ suffix, role }) => `--color-${suffix} points at ${role}`),
    ).toEqual([]);
  });

  it("registers every border role it exposes as --border-color-*", () => {
    const roles = [
      ...new Set(
        ROLE_GROUPS.flatMap((group) => group.roles)
          .map((role) => role.variable)
          .filter((variable) => variable.startsWith("--border-")),
      ),
    ];
    const registered = new Set(
      registrations("border-color").map(({ role }) => role),
    );
    expect(roles.filter((role) => !registered.has(role))).toEqual([]);
  });

  it("registers every text role it exposes as --text-color-*", () => {
    // Every `--text-*` role that is not a size — sizes share the spelling and
    // register in Tailwind's own `--text-*` namespace, which is the collision
    // typography.css documents.
    const roles = [
      ...new Set(
        ROLE_GROUPS.flatMap((group) => group.roles)
          .map((role) => role.variable)
          .filter((variable) => variable.startsWith("--text-")),
      ),
    ];
    const registered = new Set(
      registrations("text-color").map(({ role }) => role),
    );
    expect(roles.filter((role) => !registered.has(role))).toEqual([]);
  });
});
