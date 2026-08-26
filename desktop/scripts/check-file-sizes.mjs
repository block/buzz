import path from "node:path";
import { fileURLToPath } from "node:url";
import { runFileSizeCheck } from "../../scripts/check-file-sizes-core.mjs";

const __dirname = path.dirname(fileURLToPath(import.meta.url));
const projectRoot = path.resolve(__dirname, "..");

// Raised from 1000 to 1300 (0.5.20-1). The 1000-line cap forced brittle
// `biome-ignore` line-squashing on a handful of monolithic hooks/components and
// fired on every upstream catch-up against upstream's own file growth, where
// extraction just re-conflicts on the next merge. A file already at/over the
// baseline still may not grow past its baseline (see allowedLineCount), so this
// only widens the ceiling for genuinely new growth, not the ratchet itself.
const MAX_LINES = 1300;

const rules = [
  { root: "src-tauri/src", extensions: new Set([".rs"]), maxLines: MAX_LINES },
  // Workspace member crates. Without this the ratchet's only Rust root is
  // `src-tauri/src`, and a crate under `src-tauri/crates/` is born outside the
  // repo's one size discipline -- silently, since the check still exits 0.
  {
    root: "src-tauri/crates",
    extensions: new Set([".rs"]),
    maxLines: MAX_LINES,
  },
  {
    root: "src/app",
    extensions: new Set([".ts", ".tsx"]),
    maxLines: MAX_LINES,
  },
  {
    root: "src/features",
    extensions: new Set([".ts", ".tsx"]),
    maxLines: MAX_LINES,
  },
  {
    root: "src/shared/api",
    extensions: new Set([".ts", ".tsx"]),
    maxLines: MAX_LINES,
  },
  {
    root: "src/shared/context",
    extensions: new Set([".ts", ".tsx"]),
    maxLines: MAX_LINES,
  },
  {
    root: "src/shared/lib",
    extensions: new Set([".ts", ".tsx"]),
    maxLines: MAX_LINES,
  },
  {
    root: "src/shared/ui",
    extensions: new Set([".ts", ".tsx"]),
    maxLines: MAX_LINES,
  },
  {
    root: "src/shared/styles",
    extensions: new Set([".css"]),
    maxLines: MAX_LINES,
  },
];

await runFileSizeCheck({
  projectRoot,
  rules,
  label: "Desktop",
});
