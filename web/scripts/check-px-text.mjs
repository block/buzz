import path from "node:path";
import { fileURLToPath } from "node:url";
import { runPxTextCheck } from "../../scripts/check-px-text-core.mjs";

const __dirname = path.dirname(fileURLToPath(import.meta.url));
const projectRoot = path.resolve(__dirname, "..");

// Enforces the rem-token text scale, same as the desktop app. Web already runs
// the other two shared guards (file sizes, pubkey truncation); this wires up
// the third so arbitrary `text-[…px]` / `text-[…rem]` literals cannot drift
// back in. Sizes belong in `tailwind.config.js` as rem tokens so Cmd +/- zoom,
// which scales the root <html> font-size, keeps scaling the text with them.
const rules = [
  {
    root: "src",
    extensions: new Set([".ts", ".tsx", ".css"]),
  },
];

// Decorative / chrome exceptions: `relativePath:matchedLiteral`. None yet —
// web has no fixed-size display glyphs of the kind desktop allowlists.
const overrides = new Set();

await runPxTextCheck({
  projectRoot,
  rules,
  overrides,
  label: "Web",
  scriptPath: "web/scripts/check-px-text.mjs",
});
