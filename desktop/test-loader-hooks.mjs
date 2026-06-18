import { fileURLToPath } from "node:url";
import path from "node:path";

const srcRoot = path.resolve(
  path.dirname(fileURLToPath(import.meta.url)),
  "src",
);

const repoRoot = path.resolve(
  path.dirname(fileURLToPath(import.meta.url)),
  "..",
);

export function resolve(specifier, context, nextResolve) {
  if (specifier === "@features-manifest") {
    const resolved = path.join(repoRoot, "preview-features.json");
    return nextResolve(resolved, context);
  }
  if (specifier.startsWith("@/")) {
    const stripped = specifier.slice(2);
    // Preserve explicit extensions (.mjs, .js, .json, .ts, etc.). The bundler
    // tolerates extensionless `@/` imports for .ts files; node's ESM resolver
    // does not, so we only synthesize `.ts` when the specifier has no
    // extension. Otherwise paths like `@/.../foo.mjs` would be coerced into
    // `foo.mjs.ts` and fail to resolve.
    const resolved = path.extname(stripped)
      ? `${srcRoot}/${stripped}`
      : `${srcRoot}/${stripped}.ts`;
    return nextResolve(resolved, context);
  }
  // Resolve extensionless relative TS imports (e.g. `./parseImeta`) — the app's
  // bundler adds the extension, but node's ESM resolver does not. Without this,
  // any .ts that relative-imports a sibling .ts can't be imported from a test,
  // which previously forced stale inlined copies of the source under test.
  if (
    (specifier.startsWith("./") || specifier.startsWith("../")) &&
    !path.extname(specifier) &&
    context.parentURL
  ) {
    const resolved = new URL(`${specifier}.ts`, context.parentURL).href;
    return nextResolve(resolved, context);
  }
  return nextResolve(specifier, context);
}
