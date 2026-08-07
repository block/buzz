import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import test from "node:test";

const localesDirectory = join(
  dirname(fileURLToPath(import.meta.url)),
  "locales",
);
const localeNames = ["en", "zh-Hans", "zh-Hant"];

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
