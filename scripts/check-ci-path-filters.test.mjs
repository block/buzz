import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";

const workflow = readFileSync(
  new URL("../.github/workflows/ci.yml", import.meta.url),
  "utf8",
);

function parsePathFilters(source) {
  const lines = source.split("\n");
  const start = lines.findIndex((line) => line.trim() === "filters: |");
  assert.notEqual(start, -1, "CI workflow has no paths-filter block");

  const filtersIndent = lines[start].search(/\S/);
  const groups = new Map();
  let currentGroup;

  for (const line of lines.slice(start + 1)) {
    if (line.trim() === "") continue;

    const indent = line.search(/\S/);
    if (indent <= filtersIndent) break;

    const groupMatch = line.match(/^\s{12}([a-z][a-z-]*):\s*$/);
    if (groupMatch) {
      currentGroup = groupMatch[1];
      groups.set(currentGroup, []);
      continue;
    }

    const patternMatch = line.match(/^\s{14}- '([^']+)'\s*$/);
    if (patternMatch && currentGroup) {
      groups.get(currentGroup).push(patternMatch[1]);
    }
  }

  return groups;
}

function matchesPattern(pattern, path) {
  const candidate = pattern.startsWith("!") ? pattern.slice(1) : pattern;
  if (candidate.endsWith("/**")) {
    const directory = candidate.slice(0, -3);
    return path === directory || path.startsWith(`${directory}/`);
  }
  return path === candidate;
}

function selectsPath(patterns, path) {
  const included = patterns.some(
    (pattern) => !pattern.startsWith("!") && matchesPattern(pattern, path),
  );
  const excluded = patterns.some(
    (pattern) => pattern.startsWith("!") && matchesPattern(pattern, path),
  );
  return included && !excluded;
}

const filters = parsePathFilters(workflow);

const expectedSelections = new Map([
  [".cargo/config.toml", ["rust"]],
  ["Justfile", ["rust", "desktop", "web", "mobile"]],
  ["bin/hermit.hcl", ["rust", "desktop", "web", "mobile"]],
  [".github/workflows/ci.yml", ["rust", "desktop", "web", "mobile"]],
  ["package.json", ["desktop", "web"]],
  ["pnpm-workspace.yaml", ["desktop", "web"]],
  ["biome.json", ["desktop", "web"]],
  ["scripts/check-px-text-core.mjs", ["desktop"]],
  ["scripts/check-pubkey-truncation-core.mjs", ["desktop", "web"]],
]);

test("shared CI inputs select every affected component", () => {
  for (const [path, expectedGroups] of expectedSelections) {
    for (const group of expectedGroups) {
      assert.ok(filters.has(group), `CI path-filter group ${group} is missing`);
      assert.ok(
        selectsPath(filters.get(group), path),
        `${path} must select the ${group} CI group`,
      );
    }
  }
});
