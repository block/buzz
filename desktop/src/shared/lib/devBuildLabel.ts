/**
 * Cosmetic local-build label shown next to the app version in the sidebar
 * footer (e.g. "v0.5.5 - k2v1"), so builds installed during active dev/test
 * iteration are distinguishable from each other at a glance.
 *
 * This is deliberately separate from the official app version — it does NOT
 * read from or write to `tauri.conf.json`, `Cargo.toml`, or `package.json`.
 * Those stay exactly as released. Bump the number by hand each time you want
 * a fresh local build to look different in the UI; set to `null` to hide it
 * entirely (e.g. before a real release build).
 */
export const DEV_BUILD_LABEL: string | null = "k2v4";
