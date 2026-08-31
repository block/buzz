// Rule table for the Web file-size ratchet. See the sibling Desktop table for
// why the rules live apart from the runner.

export const MAX_LINES = 1000;

// `.mjs` is listed alongside `.ts`/`.tsx` so a future test rig or script module
// under these roots is governed from birth rather than discovered later.
export const SCRIPT_EXTENSIONS = new Set([".ts", ".tsx", ".mjs"]);

export const rules = [
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
];
