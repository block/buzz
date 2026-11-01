import assert from "node:assert/strict";
import fs from "node:fs";
import path from "node:path";
import test from "node:test";
import ts from "typescript";

const root = import.meta.dirname;
const keys = JSON.parse(
  fs.readFileSync(path.join(root, "locales/ui-keys.json"), "utf8"),
);

test("bundled experimental-feature names and descriptions have registered translations", () => {
  const manifest = JSON.parse(
    fs.readFileSync(path.join(root, "../../preview-features.json"), "utf8"),
  );
  for (const feature of manifest.features) {
    assert.ok(keys[feature.name], `${feature.id}: missing feature name`);
    assert.ok(keys[feature.description], `${feature.id}: missing description`);
  }
});

function files(directory) {
  return fs
    .readdirSync(directory, { withFileTypes: true })
    .flatMap((entry) =>
      entry.isDirectory()
        ? files(path.join(directory, entry.name))
        : [path.join(directory, entry.name)],
    );
}

test("authored translation calls register their English fallback and supply every interpolation value", () => {
  const errors = [];
  for (const file of files(root)) {
    if (!/\.tsx?$/.test(file) || /\.(test|jsdom-test)\.|\.d\.ts$/.test(file))
      continue;
    const ast = ts.createSourceFile(
      file,
      fs.readFileSync(file, "utf8"),
      ts.ScriptTarget.Latest,
      true,
      file.endsWith("tsx") ? ts.ScriptKind.TSX : ts.ScriptKind.TS,
    );
    function inspect(node) {
      if (
        ts.isCallExpression(node) &&
        ["uiT", "uiMessage"].includes(node.expression.getText(ast))
      ) {
        const source = node.arguments[0];
        if (source && ts.isStringLiteralLike(source)) {
          const original = source.text.replace(/\s+/g, " ").trim();
          const location = `${path.relative(root, file)}:${ast.getLineAndCharacterOfPosition(node.getStart(ast)).line + 1}`;
          if (!keys[original])
            errors.push(
              `${location}: unregistered English fallback ${original}`,
            );
          const slots = [...original.matchAll(/\{(\d+)\}/g)].map(
            (match) => `p${match[1]}`,
          );
          const values = node.arguments[1];
          if (!values && slots.length)
            errors.push(`${location}: missing interpolation object`);
          if (
            values &&
            ts.isObjectLiteralExpression(values) &&
            !values.properties.some(ts.isSpreadAssignment)
          ) {
            const provided = new Set(
              values.properties.map((property) =>
                property.name
                  ?.getText(ast)
                  .replaceAll('"', "")
                  .replaceAll("'", ""),
              ),
            );
            for (const slot of slots)
              if (!provided.has(slot))
                errors.push(`${location}: missing ${slot}`);
          }
        }
      }
      ts.forEachChild(node, inspect);
    }
    inspect(ast);
  }
  assert.deepEqual(errors, []);
});
