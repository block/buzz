import assert from "node:assert/strict";
import { execFileSync, spawnSync } from "node:child_process";
import {
  copyFileSync,
  mkdirSync,
  mkdtempSync,
  readFileSync,
  realpathSync,
  rmSync,
  writeFileSync,
} from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import test from "node:test";
import { fileURLToPath } from "node:url";
import {
  chooseRust,
  classify,
  cratePackage,
  dartDefine,
  hunkRanges,
  ITERATIONS,
  infraTest,
  listPlaywright,
  MAX_PLAYWRIGHT_JOURNEYS,
  nodeTestArgs,
  parseDartReport,
  parseJunit,
  playwrightRepetitions,
  playwrightShards,
  runners,
  rustFeatureGraphs,
  rustGroups,
  rustImporters,
  rustIncludedTests,
  rustIncludes,
  rustModulePath,
  rustModules,
  rustTarget,
  rustTargetRoot,
  rustTests,
  rustWorkspaces,
  selectRust,
  selectTests,
  testBodies,
  testCalls,
} from "./flake-check.mjs";

const read = (path) => readFileSync(new URL(path, import.meta.url), "utf8");
const root = fileURLToPath(new URL("../", import.meta.url));
const git = (args, env = {}) =>
  execFileSync("git", ["-c", "core.hooksPath=/dev/null", ...args], {
    cwd: root,
    encoding: "utf8",
    env: { ...process.env, ...env },
  });
// `select` over a synthetic head tree: HEAD's tree with a comment appended to
// each given file, so only those files differ and no test body changes. The
// outputs are keyed per runner as the workflow reads them. The selector
// writes its summary and outputs to $GITHUB_STEP_SUMMARY and $GITHUB_OUTPUT
// when they are set (as under Actions, where this suite also runs) and to
// stdout otherwise, so the child gets one contract or the other by `github`,
// never whichever the parent happens to have.
function selectAppending(files, { github = false } = {}) {
  const dir = mkdtempSync(join(tmpdir(), "flake-select-"));
  try {
    const env = { GIT_INDEX_FILE: join(dir, "index") };
    git(["read-tree", "HEAD"], env);
    for (const file of files) {
      const blob = execFileSync("git", ["hash-object", "-w", "--stdin"], {
        cwd: root,
        encoding: "utf8",
        input: `${git(["show", `HEAD:${file}`])}// flake-check probe\n`,
      }).trim();
      git(["update-index", "--cacheinfo", `100644,${blob},${file}`], env);
    }
    const tree = git(["write-tree"], env).trim();
    const { GITHUB_STEP_SUMMARY, GITHUB_OUTPUT, ...inherited } = process.env;
    const routed = {
      GITHUB_STEP_SUMMARY: join(dir, "summary.md"),
      GITHUB_OUTPUT: join(dir, "output.txt"),
    };
    const result = spawnSync(
      process.execPath,
      [join(root, "scripts/flake-check.mjs"), "select", "HEAD", tree],
      {
        cwd: root,
        encoding: "utf8",
        env: github ? { ...inherited, ...routed } : inherited,
      },
    );
    assert.equal(result.status, 0, result.stderr);
    const summary = github
      ? readFileSync(routed.GITHUB_STEP_SUMMARY, "utf8")
      : result.stdout;
    const output = github
      ? readFileSync(routed.GITHUB_OUTPUT, "utf8")
      : result.stdout;
    const outputs = Object.fromEntries(
      [...output.matchAll(/^(\w+)=(.*)$/gm)].map(([, key, value]) => [
        key,
        value.split(" ").filter(Boolean),
      ]),
    );
    return { summary, outputs, stdout: result.stdout };
  } finally {
    rmSync(dir, { recursive: true, force: true });
  }
}
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
    ["desktop/src-tauri/src/lib.rs", "tauri"],
    ["desktop/src-tauri/tests/rodio_mixer_diagnostic.rs", "tauri"],
    ["desktop/src-tauri/crates/buzz-terminal/src/lib.rs", "tauri"],
    ["desktop/src-tauri/crates/buzz-terminal/tests/latency.rs", "tauri"],
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
    "desktop/src-tauri/build.rs",
    "desktop/src-tauri/tests/fixtures/x.rs",
    "desktop/src-tauri/crates/buzz-terminal/Cargo.toml",
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
  // Each Rust runner names the cargo workspace its lane repeats.
  assert.deepEqual(Object.keys(rustWorkspaces), ["rust", "tauri"]);
  assert.ok(runners.includes("tauri"));
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
    ["desktop/src-tauri/src/app_state.rs", ["app_state"]],
    ["desktop/src-tauri/tests/rodio_mixer_diagnostic.rs", []],
    [
      "desktop/src-tauri/crates/buzz-terminal/src/grid/cell.rs",
      ["grid", "cell"],
    ],
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
    { chosen: [candidates[1], candidates[3]], unlisted: [] },
  );
  // An entry no candidate stands for is kept apart, not dropped.
  const gated = { path: "tests::gated", exact: "handlers::auth::tests::gated" };
  assert.deepEqual(
    chooseRust(
      [
        { path: "tests::same", exact: "elsewhere::tests::same" },
        { path: "tests::other", exact: "handlers::auth::tests::other" },
        gated,
      ],
      candidates,
    ),
    {
      chosen: [candidates[0], candidates[1], candidates[3], candidates[2]],
      unlisted: [gated],
    },
  );
});

test("Rust files build only the targets they can be part of, merged per package one whole flag each", () => {
  for (const [file, flags] of [
    ["crates/buzz-relay/src/api/mod.rs", [["--lib"], ["--bins"]]],
    ["crates/buzz-admin/src/main.rs", [["--bins"]]],
    ["crates/buzz-test-client/tests/e2e_relay.rs", [["--test", "e2e_relay"]]],
    ["crates/buzz-agent/tests/common/mod.rs", []],
    ["desktop/src-tauri/src/lib.rs", [["--lib"], ["--bins"]]],
    [
      "desktop/src-tauri/tests/rodio_mixer_diagnostic.rs",
      [["--test", "rodio_mixer_diagnostic"]],
    ],
    [
      "desktop/src-tauri/crates/buzz-terminal/tests/latency.rs",
      [["--test", "latency"]],
    ],
  ])
    assert.deepEqual(rustTarget(file), flags, file);
  // Two integration tests keep two `--test` flags: `--test codec connection`
  // would make nextest filter on a test named "connection" and repeat nothing.
  const args = (...entries) =>
    [...rustGroups(entries).groups.values()].map((group) => group.args);
  assert.deepEqual(
    args(
      "crates/buzz-audio-client/tests/codec.rs::roundtrip",
      "crates/buzz-audio-client/tests/connection.rs::connects",
    ),
    [["-p", "buzz-audio-client", "--test", "codec", "--test", "connection"]],
  );
  assert.deepEqual(
    args(
      "crates/buzz-audio-client/src/wire.rs::tests::frame",
      "crates/buzz-audio-client/src/jitter.rs::tests::order",
      "crates/buzz-audio-client/tests/codec.rs::roundtrip",
    ),
    [["-p", "buzz-audio-client", "--lib", "--bins", "--test", "codec"]],
  );
  // A custom-path file builds every target of its package; packages stay apart.
  assert.deepEqual(
    args(
      "crates/buzz-agent/tests/regressions.rs::replay",
      "crates/buzz-agent/tests/common/mod.rs::helper",
      "crates/buzz-admin/src/main.rs::tests::parses",
    ),
    [
      ["-p", "buzz-agent"],
      ["-p", "buzz-admin", "--bins"],
    ],
  );
  // The Tauri workspace's packages are addressed by its manifest, and never
  // mixed into a root-workspace invocation.
  const tauri = rustGroups([
    "desktop/src-tauri/tests/rodio_mixer_diagnostic.rs::single_queue_serializes_sources",
    "desktop/src-tauri/crates/buzz-terminal/tests/latency.rs::g3_renderer_acquire_stays_within_frame_budget",
  ]);
  assert.equal(tauri.runner, "tauri");
  assert.equal(rustWorkspaces[tauri.runner], "desktop/src-tauri/Cargo.toml");
  assert.deepEqual(
    [...tauri.groups.values()].map((group) => group.args),
    [
      ["-p", "buzz-desktop", "--test", "rodio_mixer_diagnostic"],
      ["-p", "buzz-terminal", "--test", "latency"],
    ],
  );
  assert.equal(
    rustGroups(["crates/buzz-core/src/filter.rs::tests::a"]).runner,
    "rust",
  );
  assert.throws(
    () =>
      rustGroups([
        "crates/buzz-core/src/filter.rs::tests::a",
        "desktop/src-tauri/src/lib.rs::tests::b",
      ]),
    /span workspaces/,
  );
  assert.throws(
    () => rustGroups(["desktop/src/x.rs::tests::a"]),
    /rust entry must be/,
  );
  // Packages the regular lanes test under a feature list in those graphs, in
  // order; the rest in their default graph alone. The desktop crate's
  // mesh-llm graph builds only its lib, and only for a selection reaching it.
  const graphs = (entries) =>
    [...rustGroups(entries).groups.values()][0].graphs.map(({ args }) =>
      args.join(" "),
    );
  assert.deepEqual(
    [
      "crates/buzz-feature-flags/src/launchdarkly.rs::tests::a",
      "crates/buzz-push-gateway/src/lib.rs::tests::b",
      "crates/buzz-core/src/filter.rs::tests::c",
      "desktop/src-tauri/src/lib.rs::tests::d",
      "desktop/src-tauri/tests/rodio_mixer_diagnostic.rs::e",
    ].map((entry) => graphs([entry])),
    [
      ["-p buzz-feature-flags --lib --bins --features launchdarkly"],
      [
        "-p buzz-push-gateway --lib --bins",
        "-p buzz-push-gateway --lib --bins --features personal-dev-app-attest",
      ],
      ["-p buzz-core --lib --bins"],
      [
        "-p buzz-desktop --lib --bins",
        "-p buzz-desktop --lib --features mesh-llm",
      ],
      ["-p buzz-desktop --test rodio_mixer_diagnostic"],
    ],
  );
  // Postgres-backed tests are excluded by name and binary, as the postgres-ci
  // default-filter selects them.
  assert.ok(infraTest("buzz_db", "event_store::postgres_tests::insert"));
  assert.ok(infraTest("postgres_relay", "admits"));
  assert.ok(infraTest("buzz_media", "tests::external_infra_s3::upload"));
  assert.ok(!infraTest("buzz_db", "tests::postgres_free"));
});

test("a helper under tests/ selects the integration binaries that include it", () => {
  const helper = "crates/x/tests/common/mod.rs";
  const includes = (target, ...lines) =>
    rustIncludes(target, `${lines.join("\n")}\n`, helper);
  // `mod common;` in a crate root, in any form; not in a comment or string.
  assert.ok(
    includes("crates/x/tests/a.rs", "mod common;", "#[test] fn t() {}"),
  );
  assert.ok(includes("crates/x/tests/a.rs", "pub(crate) mod   common ;"));
  assert.ok(!includes("crates/x/tests/a.rs", "// uses mod common; elsewhere"));
  assert.ok(!includes("crates/x/tests/a.rs", 'const S: &str = "mod common;";'));
  assert.ok(!includes("crates/x/tests/a.rs", "mod commonplace;", "mod other;"));
  // `#[path]` reaching the file itself or its directory's mod.rs.
  assert.ok(
    includes("crates/x/tests/a.rs", '#[path = "common/mod.rs"]', "mod shared;"),
  );
  assert.ok(
    includes(
      "crates/x/tests/a.rs",
      '#[path="../tests/common/mod.rs"] mod shared;',
    ),
  );
  assert.ok(
    !includes("crates/x/tests/a.rs", '#[path = "other/mod.rs"]', "mod shared;"),
  );
  assert.ok(
    rustIncludes(
      "crates/x/tests/a.rs",
      '#[path = "common/mod.rs"] mod shared;\n',
      "crates/x/tests/common/util.rs",
    ),
  );
  // A tests/<dir>/main.rs target owns every file under its directory.
  assert.ok(
    rustIncludes(
      "crates/x/tests/suite/main.rs",
      "",
      "crates/x/tests/suite/util.rs",
    ),
  );
  assert.ok(!rustIncludes("crates/x/tests/suite/main.rs", "", helper));
  assert.ok(
    rustIncludes(
      "crates/x/tests/suite/main.rs",
      '#[path = "../common/mod.rs"]\nmod common;\n',
      helper,
    ),
  );
  // The real importers of buzz-agent's shared harness: the files declaring
  // `mod common;`, not the suites that do not.
  assert.deepEqual(rustImporters("crates/buzz-agent/tests/common/mod.rs"), {
    importers: [
      "crates/buzz-agent/tests/anthropic_continuation.rs",
      "crates/buzz-agent/tests/fake_llm.rs",
      "crates/buzz-agent/tests/permission_boundary.rs",
      "crates/buzz-agent/tests/regressions.rs",
    ],
    scanned: true,
  });
  for (const importer of rustImporters("crates/buzz-agent/tests/common/mod.rs")
    .importers)
    assert.match(read(`../${importer}`), /^mod common;$/m, importer);
  assert.doesNotMatch(
    read("../crates/buzz-agent/tests/golden_transcripts.rs"),
    /^mod common;$/m,
  );
  // A binary's root is tests/<stem>.rs or tests/<dir>/main.rs; anything else
  // under tests/ is a module of one.
  for (const file of ["crates/x/tests/a.rs", "crates/x/tests/suite/main.rs"])
    assert.ok(rustTargetRoot(file), file);
  for (const file of [
    "crates/x/tests/common/mod.rs",
    "crates/x/tests/suite/util.rs",
    "crates/x/src/lib.rs",
    "crates/x/src/main.rs",
  ])
    assert.ok(!rustTargetRoot(file), file);
  // File modules resolve by cargo's conventions against the real tree: a
  // root's `mod common;` is tests/common/mod.rs, a non-root's `mod launch;`
  // is <stem>/launch.rs, a `#[path]` names its file, and a declaration in a
  // comment or one no file backs is left out.
  assert.deepEqual(
    rustModules(
      "crates/buzz-agent/tests/fake_llm.rs",
      "mod common;\n// mod missing;\nmod nowhere;\n",
    ),
    [{ name: "common", file: "crates/buzz-agent/tests/common/mod.rs" }],
  );
  assert.deepEqual(rustModules("crates/buzz-acp/src/acp.rs", "mod launch;\n"), [
    { name: "launch", file: "crates/buzz-acp/src/acp/launch.rs" },
  ]);
  const wrapper = "crates/buzz-acp/tests/pool_lifecycle_state.rs";
  assert.deepEqual(rustModules(wrapper, read(`../${wrapper}`)), [
    { name: "pool_lifecycle", file: "crates/buzz-acp/src/pool_lifecycle.rs" },
  ]);
  // The wrapper's tests are the included module's, under its module name:
  // the names nextest lists for the pool_lifecycle_state binary.
  const included = rustTests(read("../crates/buzz-acp/src/pool_lifecycle.rs"));
  assert.ok(included.length > 0);
  assert.deepEqual(
    rustIncludedTests(wrapper, read(`../${wrapper}`)).map((test) => test.name),
    included.map((test) => `pool_lifecycle::${test.name}`),
  );
  assert.deepEqual(
    rustIncludedTests("crates/buzz-agent/tests/fake_llm.rs", "mod common;\n"),
    [],
  );
});

test("select, through its entry point, repeats a changed Tauri test and the binaries including a changed helper", () => {
  // Reviewer probes on #8613: before, both produced an empty selection and
  // every repeat lane skipped.
  const tauri = selectAppending([
    "desktop/src-tauri/tests/rodio_mixer_diagnostic.rs",
  ]);
  assert.match(
    tauri.summary,
    /\| desktop\/src-tauri\/tests\/rodio_mixer_diagnostic\.rs \| tauri \| whole file \(shared code changed\): 3 test\(s\) \|/,
  );
  assert.deepEqual(tauri.outputs.rust, []);
  assert.deepEqual(
    tauri.outputs.tauri,
    rustTests(read("../desktop/src-tauri/tests/rodio_mixer_diagnostic.rs")).map(
      (test) =>
        `desktop/src-tauri/tests/rodio_mixer_diagnostic.rs::${test.name}`,
    ),
  );
  assert.equal(tauri.outputs.tauri.length, 3);
  // Under Actions, where the Desktop JS lane runs this suite, the selector
  // routes the summary and outputs to $GITHUB_STEP_SUMMARY and
  // $GITHUB_OUTPUT: the same selection arrives there, and nothing on stdout.
  const routed = selectAppending(
    ["desktop/src-tauri/tests/rodio_mixer_diagnostic.rs"],
    { github: true },
  );
  assert.equal(routed.stdout, "");
  assert.match(
    routed.summary,
    /\| desktop\/src-tauri\/tests\/rodio_mixer_diagnostic\.rs \| tauri \| whole file \(shared code changed\): 3 test\(s\) \|/,
  );
  assert.deepEqual(routed.outputs, tauri.outputs);

  const helper = selectAppending(["crates/buzz-agent/tests/common/mod.rs"]);
  assert.match(
    helper.summary,
    /\| crates\/buzz-agent\/tests\/common\/mod\.rs \| rust \| helper for: anthropic_continuation, fake_llm, permission_boundary, regressions: \d+ test\(s\) \|/,
  );
  const binaries = new Set(
    helper.outputs.rust.map((entry) => entry.split("::")[0]),
  );
  assert.deepEqual([...binaries].sort(), [
    "crates/buzz-agent/tests/anthropic_continuation.rs",
    "crates/buzz-agent/tests/fake_llm.rs",
    "crates/buzz-agent/tests/permission_boundary.rs",
    "crates/buzz-agent/tests/regressions.rs",
  ]);
  assert.ok(helper.outputs.rust.length > 0);
  // Changed together, the helper and an importer select each test once.
  const both = selectAppending([
    "crates/buzz-agent/tests/common/mod.rs",
    "crates/buzz-agent/tests/fake_llm.rs",
  ]);
  assert.deepEqual(both.outputs.rust, [...new Set(both.outputs.rust)]);
  assert.deepEqual(both.outputs.rust.length, helper.outputs.rust.length);
  // A tests/<stem>.rs root with no test of its own is a binary, not a
  // helper: buzz-acp's pool_lifecycle_state compiles src/pool_lifecycle.rs
  // in by `#[path]`, and a change to it repeats that binary's tests, not its
  // siblings' (before, the selection was git_bootstrap, run_task and
  // stdio_contract, and pool_lifecycle_state was not repeated at all).
  const wrapper = "crates/buzz-acp/tests/pool_lifecycle_state.rs";
  assert.deepEqual(rustTests(read(`../${wrapper}`)), []);
  const included = rustTests(read("../crates/buzz-acp/src/pool_lifecycle.rs"));
  const wrapped = selectAppending([wrapper]);
  assert.match(
    wrapped.summary,
    new RegExp(
      `\\| crates/buzz-acp/tests/pool_lifecycle_state\\.rs \\| rust \\| whole file \\(shared code changed\\): ${included.length} test\\(s\\) \\|`,
    ),
  );
  assert.deepEqual(
    wrapped.outputs.rust,
    included.map((test) => `${wrapper}::pool_lifecycle::${test.name}`),
  );
  const { groups } = rustGroups(wrapped.outputs.rust);
  assert.deepEqual(groups.get("buzz-acp").args, [
    "-p",
    "buzz-acp",
    "--test",
    "pool_lifecycle_state",
  ]);
  assert.deepEqual(
    groups.get("buzz-acp").tests.map(({ exact }) => exact),
    included.map((test) => `pool_lifecycle::${test.name}`),
  );
  // A src/** file without test functions still selects nothing: the regular
  // lanes run what depends on it.
  const source = selectAppending(["crates/buzz-core/src/lib.rs"]);
  assert.match(
    source.summary,
    /\| crates\/buzz-core\/src\/lib\.rs \| rust \| no test functions \|/,
  );
  assert.deepEqual(source.outputs.rust, []);
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
  // Tests of the file modules an integration-test root declares ride along
  // with a whole selection, not with a hunk inside one of the root's own
  // tests; a root with no test of its own selects only them, and one with no
  // tests at all is not a test file.
  const included = [
    { name: "m::a", ignored: false },
    { name: "m::b", ignored: true },
  ];
  assert.deepEqual(select(diff([4, 0, 5, 1]), { included }), {
    ...whole("shared code changed"),
    names: [...whole().names, "m::a"],
  });
  assert.deepEqual(select(diff([21, 1, 21, 1]), { included }), {
    lines: [19],
    names: ["tests::plain"],
  });
  assert.deepEqual(selectRust(diff([1, 1, 1, 1]), "mod m;\n", { included }), {
    whole: true,
    reason: "shared code changed",
    names: ["m::a"],
  });
  assert.deepEqual(
    selectRust(diff([0, 0, 1, 1]), "mod m;\n", { added: true, included }),
    { whole: true, reason: "new file", names: ["m::a"] },
  );
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
  // nextest reads its config from the workspace root, so each Rust workspace
  // carries the flake profile.
  for (const config of [
    "../.config/nextest.toml",
    "../desktop/src-tauri/.config/nextest.toml",
  ])
    assert.match(
      read(config),
      /^\[profile\.flake\]\n(?:#.*\n)*retries = 0\nfail-fast = false\n\n\[profile\.flake\.junit\]\npath = "junit\.xml"\n/m,
      config,
    );
  // The infrastructure exclusion mirrors the postgres-ci default-filter.
  for (const predicate of [
    "test(/postgres_tests::/)",
    "binary(/^postgres_/)",
    "test(/(^|::)external_infra[^:]*::/)",
  ])
    assert.ok(read("../.config/nextest.toml").includes(predicate), predicate);
  // Feature graphs mirror the Justfile recipes the regular lanes run: every
  // package test-unit runs under a feature, in each graph it runs there, and
  // the desktop crate's default and lib-only mesh-llm lanes.
  const justfile = read("../Justfile");
  const graphs = {};
  for (const [, crate, features] of justfile.matchAll(
    /^ +cargo nextest run -p ([\w-]+)(?: --features ([\w,-]+))?$/gm,
  )) {
    graphs[crate] ??= [];
    graphs[crate].push({ features: features?.split(",") ?? [] });
  }
  assert.ok(
    justfile.includes(
      "\ndesktop-tauri-test: _ensure-sidecar-stubs\n    cd desktop/src-tauri && cargo test --workspace\n",
    ),
  );
  assert.ok(
    justfile.includes(
      "\ndesktop-tauri-test-mesh: _ensure-sidecar-stubs\n    cargo test --manifest-path {{desktop_tauri_manifest}} --features mesh-llm --lib\n",
    ),
  );
  graphs["buzz-desktop"] = [
    { features: [] },
    { features: ["mesh-llm"], lib: true },
  ];
  assert.deepEqual(
    rustFeatureGraphs,
    Object.fromEntries(
      Object.entries(graphs).filter(([, list]) =>
        list.some(({ features }) => features.length),
      ),
    ),
  );
  const flake = read("../.github/workflows/_ci-flake.yml");
  for (const runner of ["rust", "tauri", "node", "dart"])
    assert.ok(
      flake.includes(`flake-check.mjs ${runner} --iterations=10 $SELECTED`),
      runner,
    );
  // The Tauri lane builds what the Desktop Tauri core lane does: the same
  // workspace cache, system packages and sidecar stubs.
  const desktop = read("../.github/workflows/_ci-desktop.yml");
  const packages = (text) =>
    /apt-get install -y --no-install-recommends \\\n((?: {12}\S+(?: \\)?\n)+)/.exec(
      text,
    )?.[1];
  assert.ok(packages(desktop), "Desktop Tauri apt packages");
  assert.equal(packages(flake), packages(desktop));
  assert.match(
    flake,
    /flake-tauri:\n(?: {4}.*\n)*? {6}- uses: Swatinem\/rust-cache@\w+\n {8}with:\n {10}workspaces: desktop\/src-tauri\n {10}shared-key: desktop-core\n {10}save-if: false\n/,
  );
  assert.match(desktop, /- lane: core\n {12}cache: desktop-core\n/);
  assert.ok(flake.includes("run: just _ensure-sidecar-stubs"));
  assert.ok(read("../Justfile").includes("\n_ensure-sidecar-stubs:\n"));
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

// The repeat, through its entry point, in a scratch tree holding the script
// and the packages its entries name, with a fake `cargo` on PATH listing
// `tests` ({pkg, binary, binaryName, name, feature?, ignored?, failsWith?}; a
// feature test only under that feature) and passing every test a run selects
// but one failing in the graph enabling its `failsWith` feature.
function repeatWithFakeCargo(runner, entries, tests) {
  const dir = realpathSync(mkdtempSync(join(tmpdir(), "flake-repeat-")));
  try {
    mkdirSync(join(dir, "scripts"));
    copyFileSync(
      join(root, "scripts/flake-check.mjs"),
      join(dir, "scripts/flake-check.mjs"),
    );
    for (const [crateDir, name] of [
      ["crates/buzz-feature-flags", "buzz-feature-flags"],
      ["crates/buzz-push-gateway", "buzz-push-gateway"],
      ["desktop/src-tauri", "buzz-desktop"],
    ]) {
      mkdirSync(join(dir, crateDir, "src"), { recursive: true });
      writeFileSync(
        join(dir, crateDir, "Cargo.toml"),
        `[package]\nname = "${name}"\n`,
      );
      writeFileSync(join(dir, crateDir, "src/lib.rs"), "");
    }
    mkdirSync(join(dir, "bin"));
    writeFileSync(
      join(dir, "fake-cargo.mjs"),
      `import { appendFileSync, mkdirSync, writeFileSync } from "node:fs";
import { dirname, join } from "node:path";
const args = process.argv.slice(2);
appendFileSync(process.env.FAKE_CARGO_LOG, JSON.stringify(args) + "\\n");
const value = (flag) => args[args.indexOf(flag) + 1];
const features = args.includes("--features") ? value("--features").split(",") : [];
const tests = JSON.parse(process.env.FAKE_CARGO_TESTS).filter(
  (test) => test.pkg === value("-p") && (!test.feature || features.includes(test.feature)),
);
if (args[1] === "list") {
  const suites = {};
  for (const test of tests) {
    suites[test.binary] ??= { "binary-name": test.binaryName, testcases: {} };
    suites[test.binary].testcases[test.name] = {
      ignored: Boolean(test.ignored),
      "filter-match": test.ignored ? { status: "mismatch", reason: "ignored" } : { status: "matches" },
    };
  }
  process.stdout.write(JSON.stringify({ "rust-suites": suites }));
} else {
  const junit = join(
    args.includes("--manifest-path") ? dirname(value("--manifest-path")) : ".",
    "target/nextest/flake/junit.xml",
  );
  mkdirSync(dirname(junit), { recursive: true });
  const cases = [...value("-E").matchAll(/binary_id\\(=([^)]+)\\) & test\\(=([^)]+)\\)/g)];
  const fails = ([, binary, name]) =>
    tests.some((test) => test.binary === binary && test.name === name && features.includes(test.failsWith));
  writeFileSync(junit, "<testsuites>" + cases.map((match) => '<testcase name="' + match[2] + '" classname="' + match[1] + '">' + (fails(match) ? "<failure/>" : "") + "</testcase>").join("") + "</testsuites>");
  if (cases.some(fails)) process.exitCode = 100;
}
`,
    );
    writeFileSync(
      join(dir, "bin/cargo"),
      `#!/bin/sh\nexec "${process.execPath}" "${join(dir, "fake-cargo.mjs")}" "$@"\n`,
      { mode: 0o755 },
    );
    const { GITHUB_STEP_SUMMARY, GITHUB_OUTPUT, ...inherited } = process.env;
    const log = join(dir, "cargo.log");
    const summary = join(dir, "summary.md");
    writeFileSync(summary, "");
    const result = spawnSync(
      process.execPath,
      [
        join(dir, "scripts/flake-check.mjs"),
        runner,
        "--iterations=2",
        ...entries,
      ],
      {
        cwd: dir,
        encoding: "utf8",
        env: {
          ...inherited,
          PATH: `${join(dir, "bin")}:${process.env.PATH}`,
          GITHUB_STEP_SUMMARY: summary,
          FAKE_CARGO_LOG: log,
          FAKE_CARGO_TESTS: JSON.stringify(tests),
        },
      },
    );
    assert.equal(result.signal, null, result.stderr);
    return {
      status: result.status,
      summary: readFileSync(summary, "utf8"),
      calls: readFileSync(log, "utf8")
        .trim()
        .split("\n")
        .map((line) => JSON.parse(line)),
    };
  } finally {
    rmSync(dir, { recursive: true, force: true });
  }
}

test("feature-gated Rust tests repeat in every graph the regular lanes test them in, and unlisted ones fail the lane", () => {
  // Reviewer probes on #8613: the repeat listed only the default graph, so a
  // selected `#[cfg(feature = …)]` test was dropped and the lane passed as
  // "nothing to repeat"; then it repeated each test in the first graph listing
  // it alone, so a changed `#[cfg(feature = …)]` arm inside a test the default
  // graph lists too was never compiled, and the lane passed.
  const features = (call) =>
    call.includes("--features") ? call[call.indexOf("--features") + 1] : "";
  const targets = (call) =>
    call.filter((arg) => arg === "--lib" || arg === "--bins").join(" ");
  const flags = {
    pkg: "buzz-feature-flags",
    binary: "buzz-feature-flags",
    binaryName: "buzz_feature_flags",
  };
  const root = repeatWithFakeCargo(
    "rust",
    [
      "crates/buzz-feature-flags/src/lib.rs::tests::plain",
      "crates/buzz-feature-flags/src/launchdarkly.rs::tests::null_events",
    ],
    [
      { ...flags, name: "tests::plain" },
      {
        ...flags,
        name: "launchdarkly::tests::null_events",
        feature: "launchdarkly",
      },
    ],
  );
  assert.equal(root.status, 0, root.summary);
  // buzz-feature-flags is only tested with LaunchDarkly, so it is listed and
  // run in that graph alone.
  assert.deepEqual(
    root.calls.map((call) => [call[1], features(call)]),
    [
      ["list", "launchdarkly"],
      ["run", "launchdarkly"],
      ["run", "launchdarkly"],
    ],
  );
  assert.match(
    root.summary,
    /## Flake check \(Rust\): 0 of 2 iterations failed/,
  );
  assert.match(
    root.summary,
    /\| buzz-feature-flags launchdarkly::tests::null_events \[launchdarkly\] \| ✅ \| ✅ \| 0\/2 \|/,
  );
  assert.match(
    root.summary,
    /\| buzz-feature-flags tests::plain \[launchdarkly\] \| ✅ \| ✅ \| 0\/2 \|/,
  );

  // buzz-push-gateway is tested without and with personal-dev-app-attest, and
  // config.rs's attestation_environment_is_explicit_and_build_gated asserts
  // differently in each: it repeats in both graphs, and a change breaking only
  // the feature's arm fails the lane on that graph's row while the default
  // graph's row passes.
  const gateway = {
    pkg: "buzz-push-gateway",
    binary: "buzz-push-gateway",
    binaryName: "buzz_push_gateway",
  };
  const gated = repeatWithFakeCargo(
    "rust",
    [
      "crates/buzz-push-gateway/src/config.rs::tests::attestation_environment_is_explicit_and_build_gated",
    ],
    [
      {
        ...gateway,
        name: "config::tests::attestation_environment_is_explicit_and_build_gated",
        failsWith: "personal-dev-app-attest",
      },
    ],
  );
  assert.equal(gated.status, 1, gated.summary);
  assert.deepEqual(
    gated.calls.map((call) => [call[1], features(call)]),
    [
      ["list", ""],
      ["list", "personal-dev-app-attest"],
      ["run", ""],
      ["run", "personal-dev-app-attest"],
      ["run", ""],
      ["run", "personal-dev-app-attest"],
    ],
  );
  assert.match(
    gated.summary,
    /## Flake check \(Rust\): 2 of 2 iterations failed/,
  );
  assert.match(
    gated.summary,
    /\| buzz-push-gateway config::tests::attestation_environment_is_explicit_and_build_gated \| ✅ \| ✅ \| 0\/2 \|/,
  );
  assert.match(
    gated.summary,
    /\| buzz-push-gateway config::tests::attestation_environment_is_explicit_and_build_gated \[personal-dev-app-attest\] \| ❌ \| ❌ \| 2\/2 \|/,
  );

  // The desktop crate's lib is tested in its default graph and under
  // mesh-llm, which builds the lib alone: a lib test repeats in both, a mesh
  // test in the mesh graph, and one no graph lists fails the lane by name
  // while the rest still repeat. An ignored test is excluded, not unlisted.
  const desktop = {
    pkg: "buzz-desktop",
    binary: "buzz-desktop",
    binaryName: "buzz_lib",
  };
  const tauriTests = [
    { ...desktop, name: "tests::plain" },
    { ...desktop, name: "tests::skipped", ignored: true },
    {
      ...desktop,
      name: "managed_agents::relay_mesh::tests::mesh_only",
      feature: "mesh-llm",
    },
    {
      ...desktop,
      binary: "buzz-desktop::rodio_mixer_diagnostic",
      binaryName: "rodio_mixer_diagnostic",
      name: "mixes",
    },
  ];
  const tauri = repeatWithFakeCargo(
    "tauri",
    [
      "desktop/src-tauri/src/lib.rs::tests::plain",
      "desktop/src-tauri/src/lib.rs::tests::skipped",
      "desktop/src-tauri/src/managed_agents/relay_mesh.rs::tests::mesh_only",
      "desktop/src-tauri/src/lib.rs::tests::macos_only",
    ],
    tauriTests,
  );
  assert.equal(tauri.status, 1, tauri.summary);
  assert.deepEqual(
    tauri.calls.map((call) => [call[1], features(call), targets(call)]),
    [
      ["list", "", "--lib --bins"],
      ["list", "mesh-llm", "--lib"],
      ["run", "", "--lib --bins"],
      ["run", "mesh-llm", "--lib"],
      ["run", "", "--lib --bins"],
      ["run", "mesh-llm", "--lib"],
    ],
  );
  assert.match(
    tauri.summary,
    /## Flake check \(Desktop Tauri\): 1 selected test\(s\) not listed\n[^#]*- `desktop\/src-tauri\/src\/lib\.rs::tests::macos_only`\n/,
  );
  assert.doesNotMatch(tauri.summary, /skipped`/);
  assert.match(
    tauri.summary,
    /## Flake check \(Desktop Tauri\): 0 of 2 iterations failed/,
  );
  for (const row of [
    "managed_agents::relay_mesh::tests::mesh_only [mesh-llm]",
    "tests::plain",
    "tests::plain [mesh-llm]",
  ])
    assert.ok(
      tauri.summary.includes(`| buzz-desktop ${row} | ✅ | ✅ | 0/2 |`),
      row,
    );
  assert.doesNotMatch(tauri.summary, /mesh_only \|/);

  // An integration test selection never reaches the lib-only mesh graph.
  const integration = repeatWithFakeCargo(
    "tauri",
    ["desktop/src-tauri/tests/rodio_mixer_diagnostic.rs::mixes"],
    tauriTests,
  );
  assert.equal(integration.status, 0, integration.summary);
  assert.deepEqual(
    integration.calls.map((call) => [call[1], features(call)]),
    [
      ["list", ""],
      ["run", ""],
      ["run", ""],
    ],
  );
  assert.ok(
    integration.summary.includes(
      "| buzz-desktop::rodio_mixer_diagnostic mixes | ✅ | ✅ | 0/2 |",
    ),
  );

  // A selection of only ignored tests is excluded, and passes as such; one
  // of only unlisted tests fails without running anything.
  const ignored = repeatWithFakeCargo(
    "tauri",
    ["desktop/src-tauri/src/lib.rs::tests::skipped"],
    tauriTests,
  );
  assert.equal(ignored.status, 0, ignored.summary);
  assert.match(
    ignored.summary,
    /## Flake check \(Desktop Tauri\): nothing to repeat/,
  );
  const unlisted = repeatWithFakeCargo(
    "tauri",
    ["desktop/src-tauri/src/lib.rs::tests::macos_only"],
    tauriTests,
  );
  assert.equal(unlisted.status, 1, unlisted.summary);
  assert.deepEqual(
    unlisted.calls.map((call) => call[1]),
    ["list", "list"],
  );
  assert.doesNotMatch(unlisted.summary, /nothing to repeat/);
});
