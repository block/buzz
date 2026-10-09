#!/usr/bin/env node
// Fails when Swatinem/rust-cache would mistake dependencies for workspace
// members. rust-cache keeps target/ outputs only for packages whose
// manifest_path is outside the workspace root; a dependency whose manifest
// sits under the root (a registry under the checkout, a vendored source, a
// path dependency into a nested directory) is treated as workspace code and
// cleaned away. When that covers every dependency the root-workspace cache
// saves an empty target/, and the exact-key hit blocks every later save.
//
// Usage: cargo metadata --all-features --format-version 1 | node scripts/check-rust-cache-workspace-root.mjs
import { readFileSync } from "node:fs";
import { sep } from "node:path";
import { fileURLToPath } from "node:url";

export function dependenciesUnderWorkspaceRoot(metadata) {
  const members = new Set(metadata.workspace_members);
  const root = metadata.workspace_root.endsWith(sep)
    ? metadata.workspace_root
    : metadata.workspace_root + sep;
  return metadata.packages
    .filter((pkg) => !members.has(pkg.id) && pkg.manifest_path.startsWith(root))
    .map((pkg) => `${pkg.name} ${pkg.version} (${pkg.manifest_path})`);
}

if (process.argv[1] === fileURLToPath(import.meta.url)) {
  const metadata = JSON.parse(readFileSync(0, "utf8"));
  const inside = dependenciesUnderWorkspaceRoot(metadata);
  if (inside.length > 0) {
    console.error(
      `${inside.length} non-member package(s) have a manifest under ${metadata.workspace_root}; ` +
        "rust-cache would treat them as workspace code and save an empty target/. " +
        "Keep CARGO_HOME and vendored sources outside the checkout (see bin/hermit.hcl).",
    );
    for (const line of inside.slice(0, 10)) console.error(`  ${line}`);
    process.exit(1);
  }
  const outside = metadata.packages.length - metadata.workspace_members.length;
  console.log(`rust-cache workspace root ok: ${outside} dependency package(s) outside ${metadata.workspace_root}`);
}
