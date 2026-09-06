import { useEffect, useState } from "react";

/**
 * Resolves what a token points at and what it finally paints, by reading the
 * live CSS rather than a hand-written copy of it.
 *
 * The alternative is a hex column typed into the registry, which is a second
 * source of truth for the same fact: edit a ramp in `tokens.css` and the
 * documentation is silently wrong. Reading the real CSS means the table reports
 * what the product would actually paint, in whichever mode is active.
 *
 * This needs **two** different reads, because neither alone can answer both
 * questions:
 *
 *   - `getComputedStyle().getPropertyValue('--bg-panel')` returns the *resolved*
 *     value, `#ffffff`. Good for the final value, useless for the chain — the
 *     `var(--neutral-1)` indirection is already gone by the time you can read it.
 *   - Walking `document.styleSheets` finds the *declared* value,
 *     `var(--neutral-1)`, which is the only place the chain still exists.
 *
 * So: CSSOM for "what does this point at", computed style for "what colour is
 * that". Getting this wrong is invisible — every Base cell just reads `—`, which
 * looks like a design choice rather than a broken lookup.
 */

/** Matches a declaration that is exactly one `var()` reference. */
const SINGLE_VAR = /^var\(\s*(--[\w-]+)\s*(?:,[^)]*)?\)$/;

export interface ResolvedToken {
  /** The custom property, e.g. `--bg-panel`. */
  variable: string;
  /** Its declared value: `var(--neutral-1)`, or a literal. */
  declared: string;
  /** The single variable it points at, when it points at exactly one. */
  pointsAtVariable: string | null;
  /** The final painted value: `#ffffff`, a gradient, a colour-mix. */
  value: string;
}

/**
 * Collects declared values for custom properties from the document's own
 * stylesheets.
 *
 * Later rules win, matching the cascade for the equal-specificity `:root` and
 * `.dark` selectors this system uses. Dark-mode declarations are skipped unless
 * dark is active, so the chain reported matches the mode on screen.
 */
function readDeclaredValues(isDark: boolean): Map<string, string> {
  const declared = new Map<string, string>();

  for (const sheet of Array.from(document.styleSheets)) {
    let rules: CSSRuleList;
    try {
      rules = sheet.cssRules;
    } catch {
      // A cross-origin sheet throws on access. None of ours are, and a font
      // stylesheet holds no tokens, so skipping is correct rather than lossy.
      continue;
    }

    for (const rule of Array.from(rules)) {
      if (!(rule instanceof CSSStyleRule)) continue;

      const selector = rule.selectorText;
      const isDarkRule = selector.includes(".dark");
      // A `.dark` rule only describes the page when dark is active.
      if (isDarkRule && !isDark) continue;

      for (const property of Array.from(rule.style)) {
        if (property.startsWith("--")) {
          declared.set(property, rule.style.getPropertyValue(property).trim());
        }
      }
    }
  }

  return declared;
}

/**
 * Resolves a list of custom properties, re-running whenever the colour scheme
 * changes.
 */
export function useResolvedTokens(
  variables: string[],
): Map<string, ResolvedToken> {
  const [resolved, setResolved] = useState<Map<string, ResolvedToken>>(
    () => new Map(),
  );

  // Callers build this array by mapping the registry, so it is a fresh array
  // every render. Key the effect off its content instead of its identity.
  const key = variables.join(",");

  useEffect(() => {
    const root = document.documentElement;

    const read = () => {
      const isDark = root.classList.contains("dark");
      const declaredValues = readDeclaredValues(isDark);
      const computed = getComputedStyle(root);

      const next = new Map<string, ResolvedToken>();
      for (const variable of key.split(",")) {
        if (variable === "") continue;

        const declared = declaredValues.get(variable) ?? "";
        const match = SINGLE_VAR.exec(declared);

        next.set(variable, {
          variable,
          declared,
          pointsAtVariable: match ? match[1] : null,
          // The computed value is already fully resolved, which is exactly what
          // the value column wants.
          value: computed.getPropertyValue(variable).trim(),
        });
      }
      setResolved(next);
    };

    read();

    // `.dark` toggles on the root, and a class change does not otherwise tell
    // React that every colour on the page just changed.
    const observer = new MutationObserver(read);
    observer.observe(root, { attributeFilter: ["class", "style"] });
    return () => observer.disconnect();
  }, [key]);

  return resolved;
}

/** `--neutral-7` → `neutral 7`, for display. */
export function humanizeVariable(variable: string): string {
  return variable.replace(/^--/, "").replace(/-/g, " ");
}

const HEX = /^#[0-9a-f]{3,8}$/i;

export function isHex(value: string): boolean {
  return HEX.test(value);
}

/** Labels what kind of value this is, when it is not a plain hex. */
export function describeValueKind(value: string): string {
  if (HEX.test(value)) return "hex";
  if (/^rgba?\(/i.test(value)) return "rgba";
  if (/^hsla?\(/i.test(value)) return "hsl";
  if (/gradient\(/i.test(value)) return "gradient";
  if (/^color-mix\(/i.test(value)) return "color-mix";
  return "value";
}
