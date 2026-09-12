const DESKTOP_FRONTEND_MAX_LINES = 1200;
const DESKTOP_RUST_MAX_LINES = 1500;

export const rules = [
  {
    root: "src-tauri/src",
    extensions: new Set([".rs"]),
    maxLines: DESKTOP_RUST_MAX_LINES,
  },
  // Workspace member crates. Without this the ratchet's only Rust root is
  // `src-tauri/src`, and a crate under `src-tauri/crates/` is born outside the
  // repo's one size discipline -- silently, since the check still exits 0.
  {
    root: "src-tauri/crates",
    extensions: new Set([".rs"]),
    maxLines: DESKTOP_RUST_MAX_LINES,
  },
  {
    root: "src/app",
    extensions: new Set([".ts", ".tsx"]),
    maxLines: DESKTOP_FRONTEND_MAX_LINES,
  },
  {
    root: "src/features",
    extensions: new Set([".ts", ".tsx"]),
    maxLines: DESKTOP_FRONTEND_MAX_LINES,
  },
  {
    root: "src/shared/api",
    extensions: new Set([".ts", ".tsx"]),
    maxLines: DESKTOP_FRONTEND_MAX_LINES,
  },
  {
    root: "src/shared/context",
    extensions: new Set([".ts", ".tsx"]),
    maxLines: DESKTOP_FRONTEND_MAX_LINES,
  },
  {
    root: "src/shared/lib",
    extensions: new Set([".ts", ".tsx"]),
    maxLines: DESKTOP_FRONTEND_MAX_LINES,
  },
  {
    root: "src/shared/ui",
    extensions: new Set([".ts", ".tsx"]),
    maxLines: DESKTOP_FRONTEND_MAX_LINES,
  },
  {
    root: "src/shared/styles",
    extensions: new Set([".css"]),
    maxLines: DESKTOP_FRONTEND_MAX_LINES,
  },
];
