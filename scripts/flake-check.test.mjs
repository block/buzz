import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";
import {
  chooseRust,
  classify,
  cratePackage,
  dartDefine,
  hunkRanges,
  ITERATIONS,
  listPlaywright,
  MAX_PLAYWRIGHT_JOURNEYS,
  nodeTestArgs,
  parseDartReport,
  parseJunit,
  playwrightRepetitions,
  playwrightShards,
  rustModulePath,
  rustTarget,
  rustTests,
  selectRust,
  selectTests,
  testBodies,
  testCalls,
} from "./flake-check.mjs";

const read = (path) => readFileSync(new URL(path, import.meta.url), "utf8");
// A hunk writing new-side lines start..end; by default it removed nothing.
const hunk = (start, end, removed = { start: start + 1, end: start }) => ({
  start,
  end,
  removed,
});
// A -U0 diff of (old start, old count, new start, new count) hunks.
const diff = (...hunks) =>
  hunks.map(([o, oc, n, nc]) => `@@ -${o},${oc} +${n},${nc} @@`).join("\n");

test("classification follows each runner's test file layout", () => {
  for (const [file, runner] of [
    ["crates/buzz-core/src/filter.rs", "rust"],
    ["crates/buzz-test-client/tests/e2e_relay.rs", "rust"],
    ["desktop/src/features/channels/lib/channelDescription.test.mjs", "node"],
    ["desktop/src/shared/ui/thing.jsdom-test.mjs", "node"],
    ["desktop/scripts/demo-build-config.test.mjs", "node"],
    ["desktop/tests/e2e/channels.spec.ts", "playwright"],
    ["mobile/test/features/invites/invite_join_provider_test.dart", "dart"],
  ])
    assert.equal(classify(file), runner, file);
  for (const file of [
    "crates/buzz-core/Cargo.toml",
    "crates/buzz-feature-flags/tests/fixtures/relay-feature-selection/src/main.rs",
    "desktop/src-tauri/src/lib.rs",
    "desktop/src/features/channels/lib/channelDescription.ts",
    "desktop/tests/e2e/helpers/animations.ts",
    "desktop/tests/e2e/typing-latency.perf.ts",
    "desktop/tests/e2e/fixtures/x.spec.ts",
    "mobile/lib/main.dart",
    "mobile/test/helpers.dart",
    "scripts/flake-check.test.mjs",
    "web/src/x.test.mjs",
    // Selections are shell-split downstream; refuse unusual names.
    "desktop/tests/e2e/odd name.spec.ts",
    "crates/buzz-core/src/$(id).rs",
  ])
    assert.equal(classify(file), undefined, file);
});

test("-U0 hunks map to new-side ranges, with deletions attached to the preceding line and their old-side lines kept", () => {
  const text = [
    "diff --git a/x.spec.ts b/x.spec.ts",
    "--- a/x.spec.ts",
    "+++ b/x.spec.ts",
    "@@ -1 +1 @@",
    '-import { test } from "@playwright/test";',
    '+import { expect, test } from "@playwright/test";',
    "@@ -12,0 +13,3 @@ test('a', async () => {",
    "+  const x = 1;",
    "+  const y = 2;",
    "+  expect(x).toBe(y - 1);",
    "@@ -30,2 +33,0 @@ test('b', async () => {",
    "-  removed();",
    "-  alsoRemoved();",
    "@@ -1,3 +0,0 @@",
    "-gone",
    "",
  ].join("\n");
  assert.deepEqual(hunkRanges(text), [
    { start: 1, end: 1, removed: { start: 1, end: 1 } },
    { start: 13, end: 15, removed: { start: 12, end: 11 } },
    { start: 33, end: 33, removed: { start: 30, end: 31 } },
    { start: 0, end: 0, removed: { start: 1, end: 3 } },
  ]);
  assert.deepEqual(hunkRanges(""), []);
});

test("Playwright test calls span the declaration; groups, hooks and configuration are outside", () => {
  const spec = [
    'import { expect, test } from "@playwright/test";',
    "test.use({ viewport: { width: 800, height: 600 } });",
    "const seed = 1;",
    'test.describe("suite", () => {',
    "  test.beforeEach(async ({ page }) => {",
    '    await page.goto("/");',
    "  });",
    '  test("journey", async ({ page }) => {',
    '    test.skip(seed === 2, "not here");',
    '    await expect(page.getByText("Go")).toBeVisible();',
    "  });",
    '  for (const mode of ["mouse", "keyboard"]) {',
    '    test.fixme(mode + " later", async () => {',
    "      await Promise.resolve();",
    "    });",
    "  }",
    "});",
    'test.describe.configure({ mode: "serial" });',
    'test.fail("known broken", async () => {',
    "  await Promise.resolve();",
    "});",
    "",
  ].join("\n");
  const calls = testCalls(spec, "x.spec.ts");
  assert.deepEqual(calls, [
    { start: 8, end: 11 },
    { start: 13, end: 15 },
    { start: 19, end: 21 },
  ]);
  // Playwright lists a looped declaration once per iteration, at one line.
  assert.deepEqual(testBodies(calls, [13, 13, 8]), [
    { start: 8, end: 11, line: 8 },
    { start: 13, end: 15, line: 13 },
  ]);
  // A listed line outside every call (a hook) has no body.
  assert.deepEqual(testBodies(calls, [5]), []);
});

const rustSource = [
  "//! Module docs mentioning #[test] in prose.",
  "use std::fmt;",
  "",
  "pub fn code_under_test(input: &str) -> String {",
  "    let brace = '{';",
  '    let raw = r"\\";',
  '    format!("{brace}{raw}{input}")',
  "}",
  "",
  "#[cfg(test)]",
  "mod tests {",
  "    use super::*;",
  "",
  "    fn helper() -> &'static str {",
  '        "}"',
  "    }",
  "",
  "    /// Docs for the first test.",
  "    #[test]",
  "    fn plain() {",
  '        assert_eq!(code_under_test("x"), "{\\\\x");',
  "    }",
  "",
  '    #[tokio::test(flavor = "multi_thread")]',
  '    #[ignore = "requires Postgres }"]',
  "    async fn needs_infra() {",
  "        /* a block comment with a } brace",
  "           and #[test] inside */",
  "        helper();",
  "    }",
  "",
  "    #[ignore]",
  "    #[test]",
  "    pub(crate) fn also_ignored() {}",
  "",
  "    mod postgres_tests {",
  "        #[sqlx::test]",
  "        async fn in_nested_module(pool: Pool) -> Result<(), E> {",
  "            Ok(())",
  "        }",
  "    }",
  "}",
  "",
].join("\n");

test("Rust test functions span their attribute block to the closing brace, through strings, chars and comments, named by their inline module path", () => {
  assert.deepEqual(rustTests(rustSource), [
    { name: "tests::plain", start: 19, end: 22, ignored: false },
    { name: "tests::needs_infra", start: 24, end: 30, ignored: true },
    { name: "tests::also_ignored", start: 32, end: 34, ignored: true },
    {
      name: "tests::postgres_tests::in_nested_module",
      start: 37,
      end: 40,
      ignored: false,
    },
  ]);
  assert.deepEqual(rustTests("fn main() {}\n"), []);
});

test("same-named Rust tests in different modules are distinct, and a hunk selects only its own", () => {
  const source = [
    "mod alpha {",
    "    #[test]",
    "    fn same() {}",
    "}",
    "pub(crate) mod beta {",
    "    mod tests {",
    "        #[test]",
    "        fn same() { let _ = 1; }",
    "    }",
    "}",
    "",
  ].join("\n");
  assert.deepEqual(
    rustTests(source).map((test) => test.name),
    ["alpha::same", "beta::tests::same"],
  );
  assert.deepEqual(
    selectRust(diff([8, 1, 8, 1]), source, { previous: source }),
    {
      lines: [7],
      names: ["beta::tests::same"],
    },
  );
  // The file's path predicts the rest of the name; cargo's layout rules.
  for (const [file, modules] of [
    ["crates/buzz-relay/src/lib.rs", []],
    ["crates/buzz-relay/src/main.rs", []],
    ["crates/buzz-relay/src/api/mod.rs", ["api"]],
    ["crates/buzz-relay/src/handlers/auth.rs", ["handlers", "auth"]],
    ["crates/buzz-test-client/src/bin/mention.rs", []],
    ["crates/buzz-test-client/tests/e2e_relay.rs", []],
    ["crates/x/tests/suite/main.rs", []],
    ["crates/x/tests/suite/helpers.rs", ["helpers"]],
  ])
    assert.deepEqual(rustModulePath(file), modules, file);
  // Listed candidates: the predicted name wins; without it every suffix
  // match is kept rather than nothing.
  const candidates = [
    { binary: "buzz-relay", name: "api::relay_members::tests::same" },
    { binary: "buzz-relay", name: "handlers::auth::tests::same" },
    { binary: "buzz-relay", name: "handlers::auth::tests::other" },
    {
      binary: "buzz-relay::bin/buzz-relay",
      name: "handlers::auth::tests::same",
    },
  ];
  assert.deepEqual(
    chooseRust(
      [{ path: "tests::same", exact: "handlers::auth::tests::same" }],
      candidates,
    ),
    [candidates[1], candidates[3]],
  );
  assert.deepEqual(
    chooseRust(
      [
        { path: "tests::same", exact: "elsewhere::tests::same" },
        { path: "tests::other", exact: "handlers::auth::tests::other" },
      ],
      candidates,
    ),
    [candidates[0], candidates[1], candidates[3], candidates[2]],
  );
});

test("Rust files build only the targets they can be part of", () => {
  for (const [file, flags] of [
    ["crates/buzz-relay/src/api/mod.rs", ["--lib", "--bins"]],
    ["crates/buzz-admin/src/main.rs", ["--bins"]],
    ["crates/buzz-test-client/tests/e2e_relay.rs", ["--test", "e2e_relay"]],
    ["crates/buzz-agent/tests/common/mod.rs", []],
  ])
    assert.deepEqual(rustTarget(file), flags, file);
});

test("Rust selection: a hunk inside a test selects it, elsewhere selects every runnable test; ignored tests never run", () => {
  const select = (text, options) =>
    selectRust(text, rustSource, { previous: rustSource, ...options });
  assert.deepEqual(select(diff([21, 1, 21, 1])), {
    lines: [19],
    names: ["tests::plain"],
  });
  // Inside an ignored test: owned, so not shared code, but nothing to run.
  assert.deepEqual(select(diff([28, 0, 29, 1])), { lines: [24], names: [] });
  // The code under test, a helper, or a deleted helper (attached to line 13)
  // can change every test.
  const whole = (reason) => ({
    whole: true,
    reason,
    names: ["tests::plain", "tests::postgres_tests::in_nested_module"],
  });
  assert.deepEqual(select(diff([4, 0, 5, 1])), whole("shared code changed"));
  assert.deepEqual(
    select(diff([21, 1, 21, 1], [14, 0, 15, 1])),
    whole("shared code changed"),
  );
  assert.deepEqual(select(diff([14, 3, 13, 0])), whole("shared code removed"));
  assert.deepEqual(
    select(diff([21, 1, 21, 1]), { added: true, previous: "" }),
    whole("new file"),
  );
  // A file without test functions is not a test file.
  assert.equal(selectRust(diff([1, 1, 1, 1]), "fn main() {}\n"), undefined);
  assert.equal(cratePackage("crates/buzz-core/src/filter.rs"), "buzz-core");
});

test("hunks inside tests select those tests; shared code, new or unlisted files select the whole file", () => {
  const bodies = [
    { start: 10, end: 22, line: 10 },
    { start: 25, end: 37, line: 25 },
    { start: 38, end: 52, line: 40 },
  ];
  // The old file had the same tests, with a hook between the last two.
  const previous = [
    { start: 10, end: 22 },
    { start: 25, end: 37 },
    { start: 41, end: 55 },
  ];
  const select = (ranges, options) =>
    selectTests(ranges, bodies, { previous, ...options });
  // Edit inside one test, including its first and last lines.
  assert.deepEqual(select([hunk(12, 14)]), { lines: [10] });
  assert.deepEqual(select([hunk(10, 10)]), { lines: [10] });
  assert.deepEqual(select([hunk(22, 22)]), { lines: [10] });
  // A hunk across two adjacent bodies selects both, by their listed lines.
  assert.deepEqual(select([hunk(36, 39)]), { lines: [25, 40] });
  // A hunk that also covers the gap between tests, or lines after the last
  // test, touches shared code.
  for (const range of [hunk(22, 25), hunk(52, 53), hunk(400, 405)])
    assert.deepEqual(select([range]), {
      whole: true,
      reason: "shared code changed",
    });
  // Several hunks accumulate distinct tests in file order.
  assert.deepEqual(select([hunk(41, 41), hunk(11, 11), hunk(12, 12)]), {
    lines: [10, 40],
  });
  // Replacing lines inside a test, or deleting some of them (attached to the
  // preceding line), selects that test when the old lines were its own.
  assert.deepEqual(select([hunk(30, 31, { start: 30, end: 32 })]), {
    lines: [25],
  });
  assert.deepEqual(select([hunk(30, 30, { start: 31, end: 32 })]), {
    lines: [25],
  });
  // Deleting the hook that followed the second test attaches to that test's
  // closing line, but the removed lines were never a test's.
  assert.deepEqual(select([hunk(37, 37, { start: 38, end: 40 })]), {
    whole: true,
    reason: "shared code removed",
  });
  // Deletion at the top of the file (line 0) and edits before the first test
  // fall back to the whole file.
  assert.deepEqual(select([hunk(0, 0, { start: 1, end: 1 })]), {
    whole: true,
    reason: "shared code removed",
  });
  assert.deepEqual(select([hunk(12, 12), hunk(3, 3)]), {
    whole: true,
    reason: "shared code changed",
  });
  assert.deepEqual(select([hunk(12, 12)], { added: true }), {
    whole: true,
    reason: "new file",
  });
  assert.deepEqual(selectTests([hunk(12, 12)], []), {
    whole: true,
    reason: "no tests listed",
  });
  // Without old-side calls, every removal is shared code.
  assert.deepEqual(
    selectTests([hunk(30, 30, { start: 31, end: 32 })], bodies),
    { whole: true, reason: "shared code removed" },
  );
});

test("Playwright lists smoke journeys per spec once each; integration-only specs list none", () => {
  const smoke = "desktop/tests/e2e/channel-sort.spec.ts";
  const looped = "desktop/tests/e2e/claude-effort-picker.spec.ts";
  const integration = "desktop/tests/e2e/agents.spec.ts";
  // Alone, an integration spec makes Playwright exit 1 with "No tests found".
  assert.deepEqual(listPlaywright([integration]), new Map([[integration, []]]));
  const listed = listPlaywright([smoke, looped, integration]);
  assert.deepEqual(listed.get(integration), []);
  const lines = listed.get(smoke);
  assert.ok(lines.length > 0, "a smoke spec must list its journeys");
  assert.equal(
    testBodies(testCalls(read(`../${smoke}`), smoke), lines).length,
    lines.length,
    "every listed journey must be owned by a test call",
  );
  // A looped declaration is listed once per iteration at the same line.
  const loopedLines = listed.get(looped);
  assert.deepEqual(loopedLines, [...new Set(loopedLines)]);
  assert.ok(
    /^\s*for \(const .+\) \{\n\s+test\(/m.test(read(`../${looped}`)),
    "the looped spec must still declare a test inside a loop",
  );
});

test("Playwright repetitions get one shard per five journeys, at most ten, and a selection past ~19 minutes per shard skips", () => {
  assert.deepEqual(
    [0, 1, 5, 6, 18, 40, 41, 50, 51, 200].map(playwrightShards),
    [1, 1, 1, 2, 4, 8, 9, 10, 10, 10],
  );
  // 250 journey-runs per shard at ~4.5 s each, over ten shards.
  assert.equal(MAX_PLAYWRIGHT_JOURNEYS, 250);
  assert.equal(ITERATIONS, 10);
});

test("nextest's JUnit classname is the binary id its list reports", () => {
  // Captured from cargo-nextest 0.9.136 on one buzz-core test: `cargo nextest
  // list -p buzz-core --lib -E 'test(=kind::tests::no_duplicate_kind_values)'
  // --message-format json` (trimmed to the matching testcase) and the
  // target/nextest/flake/junit.xml of `cargo nextest run --profile flake` with
  // the same arguments. repeatRust keys expectations as `<binary-id> <name>`
  // from the list and reads statuses as `<classname> <name>` from the report,
  // so the two must agree.
  const list = {
    "rust-suites": {
      "buzz-core": {
        "package-name": "buzz-core",
        "binary-id": "buzz-core",
        "binary-name": "buzz_core",
        kind: "lib",
        status: "listed",
        testcases: {
          "kind::tests::no_duplicate_kind_values": {
            kind: "test",
            ignored: false,
            "filter-match": { status: "matches" },
          },
        },
      },
    },
  };
  const junit = [
    '<?xml version="1.0" encoding="UTF-8"?>',
    '<testsuites name="nextest-run" tests="1" failures="0" errors="0" uuid="01abf963-07b0-46d9-8f00-e212969d85a8" timestamp="2026-10-27T00:32:44.093+11:00" time="0.042">',
    '    <testsuite name="buzz-core" tests="1" disabled="0" errors="0" failures="0">',
    '        <testcase name="kind::tests::no_duplicate_kind_values" classname="buzz-core" timestamp="2026-10-27T00:32:44.094+11:00" time="0.040">',
    "        </testcase>",
    "    </testsuite>",
    "</testsuites>",
    "",
  ].join("\n");
  const expected = Object.entries(list["rust-suites"]).flatMap(
    ([binary, suite]) =>
      Object.keys(suite.testcases).map((name) => `${binary} ${name}`),
  );
  assert.deepEqual(
    parseJunit(junit),
    new Map(expected.map((test) => [test, "passed"])),
  );
});

test("reports give per-test statuses per iteration", () => {
  assert.deepEqual(
    parseJunit(
      [
        '<?xml version="1.0" encoding="UTF-8"?>',
        '<testsuites name="nextest-run">',
        '<testsuite name="buzz-core" tests="3">',
        '<testcase name="filter::tests::a" classname="buzz-core" time="0.1"/>',
        '<testcase name="filter::tests::b &lt;x&gt;" classname="buzz-core" time="0.1">',
        '<failure type="test failure">assertion `left == right` failed</failure>',
        "</testcase>",
        '<testcase name="filter::tests::c" classname="buzz-core"><skipped/></testcase>',
        "</testsuite>",
        '<testsuite name="buzz-core::e2e" tests="1">',
        '<testcase name="d" classname="buzz-core::e2e" time="0.1"><error/></testcase>',
        "</testsuite>",
        "</testsuites>",
      ].join("\n"),
    ),
    new Map([
      ["buzz-core filter::tests::a", "passed"],
      ["buzz-core filter::tests::b <x>", "failed"],
      ["buzz-core filter::tests::c", "skipped"],
      ["buzz-core::e2e d", "failed"],
    ]),
  );
  const event = (value) => JSON.stringify(value);
  assert.deepEqual(
    parseDartReport(
      [
        event({ type: "start", protocolVersion: "0.1.1" }),
        event({
          type: "testStart",
          test: { id: 1, name: "loading /m/test/a_test.dart", url: null },
        }),
        event({
          type: "testDone",
          testID: 1,
          result: "success",
          hidden: true,
          skipped: false,
        }),
        event({
          type: "testStart",
          test: {
            id: 2,
            name: "group renders",
            url: "file:///m/test/a_test.dart",
          },
        }),
        event({
          type: "testStart",
          test: {
            id: 3,
            name: "group fails",
            url: "file:///m/test/a_test.dart",
          },
        }),
        event({
          type: "testStart",
          test: { id: 4, name: "later", url: "file:///m/test/b_test.dart" },
        }),
        "not json",
        event({
          type: "testDone",
          testID: 2,
          result: "success",
          hidden: false,
          skipped: false,
        }),
        event({
          type: "testDone",
          testID: 3,
          result: "error",
          hidden: false,
          skipped: false,
        }),
        event({
          type: "testDone",
          testID: 4,
          result: "success",
          hidden: false,
          skipped: true,
        }),
        event({ type: "done", success: false }),
      ].join("\n"),
      "/m/",
    ),
    new Map([
      ["test/a_test.dart › group renders", "passed"],
      ["test/a_test.dart › group fails", "failed"],
      ["test/b_test.dart › later", "skipped"],
    ]),
  );
  // Playwright lists a --repeat-each journey once per repetition, in order.
  const spec = (title, line, status) => ({
    title,
    file: "channels.spec.ts",
    line,
    tests: [{ status }],
  });
  assert.deepEqual(
    playwrightRepetitions({
      suites: [
        {
          specs: [spec("a", 10, "expected"), spec("a", 10, "unexpected")],
          suites: [
            { specs: [spec("b", 30, "skipped"), spec("a", 10, "expected")] },
          ],
        },
      ],
    }),
    new Map([
      ["channels.spec.ts:10 › a", ["passed", "failed", "passed"]],
      ["channels.spec.ts:30 › b", ["skipped"]],
    ]),
  );
});

test("repeat invocations mirror the lanes they stand in for", () => {
  // node:test flags come from desktop/package.json "test"; the Flutter define
  // from the mobile test shards; nextest's flake profile must keep retries off.
  const script = JSON.parse(read("../desktop/package.json")).scripts.test;
  for (const file of ["x.test.mjs", "x.jsdom-test.mjs"])
    assert.ok(script.includes(`node ${nodeTestArgs(file).join(" ")} "`), file);
  assert.ok(
    read("../.github/workflows/_ci-clients.yml").includes(
      `scripts/mobile-test-shard.sh \${{ matrix.shard }} 3 ${dartDefine}`,
    ),
  );
  assert.match(
    read("../.config/nextest.toml"),
    /^\[profile\.flake\]\n(?:#.*\n)*retries = 0\nfail-fast = false\n\n\[profile\.flake\.junit\]\npath = "junit\.xml"\n/m,
  );
  const flake = read("../.github/workflows/_ci-flake.yml");
  for (const runner of ["rust", "node", "dart"])
    assert.ok(
      flake.includes(`flake-check.mjs ${runner} --iterations=10 $SELECTED`),
      runner,
    );
  assert.ok(flake.includes("--repeat-each=10 --retries=0"));
  // The lanes are advisory: ci.yml calls the workflow on pull requests only,
  // and no required gate depends on it.
  const ci = read("../.github/workflows/ci.yml");
  assert.match(
    ci,
    /^ {2}flake-check:\n(?: {4}.*\n)*? {4}if: github\.event_name == 'pull_request'\n(?: {4}.*\n)*? {4}uses: \.\/\.github\/workflows\/_ci-flake\.yml\n/m,
  );
  assert.doesNotMatch(ci, /needs: \[[^\]]*flake/);
});
