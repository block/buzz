import { createHash } from "node:crypto";
import fs from "node:fs";
import path from "node:path";
import ts from "typescript";

export const KOREAN_CATALOG_SOURCE = Object.freeze({
  repository: "https://github.com/Gwani-28/buzz",
  ref: "0a1e20470327270e572b2495eddb55bd726951f6",
  path: "team-patches/40-korean.patch",
  upstreamLicense: "Apache-2.0",
  patchSha256:
    "f0f48c354ca48909e229dde1e66cab9d29996a82bae4dce4877f0428e5494936",
  snapshotSha256:
    "6b5bb5edfabf67c93f7a2b1917fa10e5d1c968ed61bb3b586ab7b76603e52f63",
});

/** Load the committed literal snapshot, or verify a pinned source patch first. */
export function loadKoreanCatalog(patchPath) {
  const file =
    patchPath ??
    path.resolve(import.meta.dirname, "../src/locales/ko.source.snapshot.json");
  const bytes = fs.readFileSync(file);
  const expected = patchPath
    ? KOREAN_CATALOG_SOURCE.patchSha256
    : KOREAN_CATALOG_SOURCE.snapshotSha256;
  if (sha256(bytes) !== expected)
    throw new Error("Korean catalogue source hash mismatch");
  const table = patchPath
    ? parseKoreanCatalogPatch(bytes.toString("utf8"))
    : validateKoreanCatalog(JSON.parse(bytes.toString("utf8")));
  return { table, source: KOREAN_CATALOG_SOURCE };
}

const marker =
  "diff --git a/desktop/src/shared/i18n/ko.generated.ts b/desktop/src/shared/i18n/ko.generated.ts";

/** Decode only the catalogue's added literal data, never imported code. */
export function parseKoreanCatalogPatch(patch) {
  const start = patch.indexOf(marker);
  if (start < 0 || patch.indexOf(marker, start + marker.length) !== -1)
    throw new Error("Expected exactly one Korean catalogue diff");
  const next = patch.indexOf("\ndiff --git ", start + marker.length);
  const section = patch.slice(start, next < 0 ? undefined : next);
  const source = section
    .split("\n")
    .filter((line) => line.startsWith("+") && !line.startsWith("+++"))
    .map((line) => line.slice(1))
    .join("\n");
  const ast = ts.createSourceFile(
    "catalog.ts",
    source,
    ts.ScriptTarget.Latest,
    true,
  );
  if (ast.parseDiagnostics.length) throw new Error("Invalid catalogue syntax");
  function literal(node) {
    if (ts.isStringLiteralLike(node)) return node.text;
    if (ts.isArrayLiteralExpression(node)) return node.elements.map(literal);
    if (ts.isObjectLiteralExpression(node)) {
      const entries = [];
      const names = new Set();
      for (const property of node.properties) {
        if (
          !ts.isPropertyAssignment(property) ||
          !(
            ts.isIdentifier(property.name) ||
            ts.isStringLiteralLike(property.name)
          )
        )
          throw new Error("Non-literal catalogue property");
        const name = property.name.text;
        if (names.has(name)) throw new Error("Duplicate catalogue property");
        names.add(name);
        entries.push([name, literal(property.initializer)]);
      }
      return Object.fromEntries(entries);
    }
    throw new Error("Non-literal catalogue value");
  }
  const declarations = [];
  function visit(node) {
    if (ts.isVariableDeclaration(node) && node.name.getText(ast) === "KO_TABLE")
      declarations.push(node);
    ts.forEachChild(node, visit);
  }
  visit(ast);
  if (declarations.length !== 1)
    throw new Error("Expected one KO_TABLE declaration");
  return validateKoreanCatalog(literal(declarations[0].initializer));
}

export function validateKoreanCatalog(table) {
  if (
    !table ||
    typeof table !== "object" ||
    Array.isArray(table) ||
    !table.exact ||
    typeof table.exact !== "object" ||
    Array.isArray(table.exact) ||
    !Array.isArray(table.patterns) ||
    Object.keys(table).some((k) => !["exact", "patterns"].includes(k))
  )
    throw new Error("Invalid catalogue shape");
  if (
    Object.entries(table.exact).some(
      ([en, ko]) => !en || typeof ko !== "string" || !ko,
    ) ||
    table.patterns.some(
      (pair) =>
        !Array.isArray(pair) ||
        pair.length !== 2 ||
        pair.some((value) => typeof value !== "string" || !value),
    )
  )
    throw new Error("Catalogue messages must be non-empty string pairs");
  return table;
}

export function sha256(bytes) {
  return createHash("sha256").update(bytes).digest("hex");
}
