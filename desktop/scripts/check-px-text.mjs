import path from "node:path";
import { fileURLToPath } from "node:url";
import { runPxTextCheck } from "../../scripts/check-px-text-core.mjs";

const __dirname = path.dirname(fileURLToPath(import.meta.url));
const projectRoot = path.resolve(__dirname, "..");

// Scoped to the message-timeline / thread render path — the surface where the
// rem→px zoom regression (PR #891) landed. Readable message text here MUST use
// rem-based tokens (the stock `text-base` / `text-sm` scale, chat === base)
// so Cmd +/- zoom scales it. We intentionally do NOT sweep the whole app yet
// (decorative chrome — avatar initials, day dividers, diff-viewer labels —
// still uses px); widen these roots when that sweep happens.
const rules = [
  {
    root: "src/shared/ui",
    extensions: new Set([".ts", ".tsx"]),
    files: new Set(["markdown.tsx", "mentionChip.ts"]),
  },
  {
    root: "src/features/messages/ui",
    extensions: new Set([".tsx"]),
    files: new Set(["MessageRow.tsx"]),
  },
  {
    // `.mention-highlight` lives here and was part of the #891 px regression —
    // guard the `font-size: NNpx` form too, not just the Tailwind utility.
    root: "src/shared/styles",
    extensions: new Set([".css"]),
    files: new Set(["globals.css"]),
  },
];

// Decorative / chrome px-text exceptions: `relativePath:lineNumber`. Empty for
// now — the regression footprint is fully on rem tokens.
const overrides = new Set();

await runPxTextCheck({
  projectRoot,
  rules,
  overrides,
  label: "Desktop",
  scriptPath: "desktop/scripts/check-px-text.mjs",
});
