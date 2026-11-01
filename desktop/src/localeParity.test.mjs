import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import test from "node:test";

const localesDirectory = join(
  dirname(fileURLToPath(import.meta.url)),
  "locales",
);
const localeNames = ["en", "ko", "zh-Hans", "zh-Hant"];

function flattenStrings(value, prefix = "", output = new Map()) {
  if (typeof value === "string") {
    output.set(prefix, value);
    return output;
  }

  for (const [key, child] of Object.entries(value)) {
    flattenStrings(child, prefix ? `${prefix}.${key}` : key, output);
  }
  return output;
}

function interpolationNames(value) {
  return [...value.matchAll(/\{\{\s*([^}\s]+)\s*\}\}/g)]
    .map((match) => match[1])
    .sort();
}

async function readLocale(name) {
  const contents = await readFile(
    join(localesDirectory, `${name}.json`),
    "utf8",
  );
  return flattenStrings(JSON.parse(contents));
}

test("locale resources keep keys and interpolation variables in sync", async () => {
  const locales = new Map(
    await Promise.all(
      localeNames.map(async (name) => [name, await readLocale(name)]),
    ),
  );
  const english = locales.get("en");

  for (const [name, locale] of locales) {
    assert.deepEqual(
      [...locale.keys()].sort(),
      [...english.keys()].sort(),
      `${name} must contain the same translation keys as en`,
    );

    for (const key of english.keys()) {
      assert.deepEqual(
        interpolationNames(locale.get(key)),
        interpolationNames(english.get(key)),
        `${name}.${key} must keep the same interpolation variables as en`,
      );
    }
  }
});

test("Korean UI preserves message keys and all semantic interpolation slots", async () => {
  const [english, korean, registry, omissions] = await Promise.all(
    ["en.ui", "ko.ui", "ui-keys", "ko.grammar-slots"].map(async (name) =>
      JSON.parse(
        await readFile(join(localesDirectory, `${name}.json`), "utf8"),
      ),
    ),
  );
  assert.deepEqual(Object.keys(korean).sort(), Object.keys(english).sort());
  assert.equal(
    new Set(Object.values(registry)).size,
    Object.keys(registry).length,
  );
  for (const [key, original] of Object.entries(english)) {
    assert.equal(typeof korean[key], "string");
    assert.ok(korean[key].trim(), `${key} must not be empty`);
    const before = new Set(interpolationNames(original));
    const after = new Set(interpolationNames(korean[key]));
    const missing = [...before].filter((name) => !after.has(name)).sort();
    assert.deepEqual(
      [...after].filter((name) => !before.has(name)),
      [],
      `Unexpected slot in ${key}`,
    );
    assert.deepEqual(
      missing,
      omissions[key]?.omitted ?? [],
      `Unreviewed omitted slot in ${key}`,
    );
    if (omissions[key]) assert.ok(omissions[key].reason.length > 30);
  }
});
