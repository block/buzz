import { fileURLToPath } from "node:url";
import fs from "node:fs";
import path from "node:path";
import ts from "typescript";

const srcRoot = path.resolve(
  path.dirname(fileURLToPath(import.meta.url)),
  "src",
);

const repoRoot = path.resolve(
  path.dirname(fileURLToPath(import.meta.url)),
  "..",
);

function resolveSourcePath(basePath) {
  if (path.extname(basePath)) {
    return basePath;
  }

  for (const extension of [".ts", ".tsx", ".js", ".jsx", ".mjs"]) {
    const candidate = `${basePath}${extension}`;
    if (fs.existsSync(candidate)) {
      return candidate;
    }
  }

  for (const extension of [".ts", ".tsx", ".js", ".jsx", ".mjs"]) {
    const candidate = path.join(basePath, `index${extension}`);
    if (fs.existsSync(candidate)) {
      return candidate;
    }
  }

  return `${basePath}.ts`;
}

// emoji-mart ships a bundled CJS main that node's cjs-module-lexer cannot
// extract named exports from (`import { init } from "emoji-mart"` throws
// under node ESM even though the bundler handles it). Tests never exercise
// the picker, so serve inert stubs for the emoji-mart entrypoints.
const stubModules = new Map([
  [
    "emoji-mart",
    "export const init = () => {};\n" +
      "export const SearchIndex = { search: async () => [] };\n" +
      "export default {};\n",
  ],
  ["@emoji-mart/react", "export default function Picker() { return null; }\n"],
]);

const STUB_URL_PREFIX = "buzz-test-stub:";

export function resolve(specifier, context, nextResolve) {
  if (stubModules.has(specifier)) {
    return {
      shortCircuit: true,
      url: `${STUB_URL_PREFIX}${specifier}`,
    };
  }
  if (specifier === "@features-manifest") {
    const resolved = path.join(repoRoot, "preview-features.json");
    return nextResolve(resolved, context);
  }
  if (specifier.startsWith("@/")) {
    const stripped = specifier.slice(2);
    // Preserve explicit extensions (.mjs, .js, .json, .ts, etc.). The bundler
    // tolerates extensionless `@/` imports for source files; node's ESM
    // resolver does not, so resolve against the extensions the app uses.
    // Otherwise paths like `@/.../foo.mjs` would be coerced into `foo.mjs.ts`
    // and fail to resolve.
    const resolved = resolveSourcePath(`${srcRoot}/${stripped}`);
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
    const parentPath = fileURLToPath(context.parentURL);
    const resolved = resolveSourcePath(
      path.resolve(path.dirname(parentPath), specifier),
    );
    return nextResolve(resolved, context);
  }
  return nextResolve(specifier, context);
}

export async function load(url, context, nextLoad) {
  if (url.startsWith(STUB_URL_PREFIX)) {
    return {
      format: "module",
      shortCircuit: true,
      source: stubModules.get(url.slice(STUB_URL_PREFIX.length)) ?? "",
    };
  }

  // The app bundler loads .json imports without attributes (e.g. the bare
  // `@emoji-mart/data` entrypoint); node's ESM resolver requires
  // `with { type: "json" }` on every hop. Serve json here so transitive
  // imports from source under test don't need bundler-only semantics.
  if (url.endsWith(".json")) {
    return {
      format: "json",
      shortCircuit: true,
      source: fs.readFileSync(fileURLToPath(url), "utf8"),
    };
  }

  if (url.endsWith(".tsx")) {
    const source = fs.readFileSync(fileURLToPath(url), "utf8");
    const transpiled = ts.transpileModule(source, {
      compilerOptions: {
        jsx: ts.JsxEmit.ReactJSX,
        module: ts.ModuleKind.ESNext,
        target: ts.ScriptTarget.ES2020,
      },
      fileName: fileURLToPath(url),
    });

    return {
      format: "module",
      shortCircuit: true,
      source: transpiled.outputText,
    };
  }

  return nextLoad(url, context);
}
