import path from "node:path";
import { fileURLToPath } from "node:url";
import { runPxTextCheck } from "../../scripts/check-px-text-core.mjs";

const __dirname = path.dirname(fileURLToPath(import.meta.url));
const projectRoot = path.resolve(__dirname, "..");

// Enforces the rem-token text scale app-wide. The rem→px zoom regression
// (PR #891) landed in the message-timeline render path, but arbitrary text
// literals (`text-[…px]`, `text-[…rem]`) had drifted across the whole desktop
// app — so the guard now scans all of `src`. Readable text MUST use a rem-based
// token (the stock `text-base`/`text-sm`/`text-xs` scale, or the `text-2xs` /
// `text-3xs` meta-text tokens) so Cmd +/- zoom scales it and the size stays on
// one consolidated scale. Genuine decorative glyphs are allowlisted below.
const rules = [
  {
    root: "src",
    extensions: new Set([".ts", ".tsx", ".css"]),
  },
];

// Decorative / chrome exceptions: `relativePath:lineNumber`. The avatar emoji
// glyph is a fixed display size sized to its avatar box (not readable message
// text), so it stays as the lone documented `text-[6rem]` literal.
const overrides = new Set([
  "src/features/settings/ui/ProfileSettingsCard.tsx:573",
  "src/features/onboarding/ui/AvatarStep.tsx:89",
]);

await runPxTextCheck({
  projectRoot,
  rules,
  overrides,
  label: "Desktop",
  scriptPath: "desktop/scripts/check-px-text.mjs",
});
