// Rule table for the Desktop file-size ratchet, kept in its own module so tests
// can assert the real configuration instead of a restatement of it. The runner
// (`check-file-sizes.mjs`) stays unconditional: a module that both exports its
// rules and self-guards its execution can silently stop gating, which is the
// same class of failure this table's `.mjs` coverage exists to prevent.

export const MAX_LINES = 1000;

// Desktop's test suite is `*.test.mjs` by convention and its shared test rigs
// are plain `.mjs` modules, so listing only `.ts`/`.tsx` here left all of them
// outside the ceiling AGENTS.md documents as enforced -- inside roots this
// ratchet already governs, and silently, since the check still exits 0.
export const SCRIPT_EXTENSIONS = new Set([".ts", ".tsx", ".mjs"]);

export const rules = [
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
    extensions: SCRIPT_EXTENSIONS,
    maxLines: MAX_LINES,
  },
  {
    root: "src/features",
    extensions: SCRIPT_EXTENSIONS,
    maxLines: MAX_LINES,
  },
  {
    root: "src/shared/api",
    extensions: SCRIPT_EXTENSIONS,
    maxLines: MAX_LINES,
  },
  {
    root: "src/shared/context",
    extensions: SCRIPT_EXTENSIONS,
    maxLines: MAX_LINES,
  },
  {
    root: "src/shared/lib",
    extensions: SCRIPT_EXTENSIONS,
    maxLines: MAX_LINES,
  },
  {
    root: "src/shared/ui",
    extensions: SCRIPT_EXTENSIONS,
    maxLines: MAX_LINES,
  },
  {
    root: "src/shared/styles",
    extensions: new Set([".css"]),
    maxLines: MAX_LINES,
  },
];
