import { fileURLToPath } from "node:url";
import path from "node:path";

const srcRoot = path.resolve(
  path.dirname(fileURLToPath(import.meta.url)),
  "src",
);

export function resolve(specifier, context, nextResolve) {
  if (specifier.startsWith("@/")) {
    const resolved = `${srcRoot}/${specifier.slice(2)}.ts`;
    return nextResolve(resolved, context);
  }
  return nextResolve(specifier, context);
}
