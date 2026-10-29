// Advisory pull-request flake check. `select` maps a diff to the tests it
// changed, per runner; `rust`, `node` and `dart` repeat a selection and report
// every iteration. Playwright repeats natively with --repeat-each, and
// `playwright-summary` tabulates its JSON report afterwards.
import { spawnSync } from "node:child_process";
import {
  appendFileSync,
  copyFileSync,
  existsSync,
  mkdirSync,
  readdirSync,
  readFileSync,
  rmSync,
} from "node:fs";
import { createRequire } from "node:module";
import { posix as path } from "node:path";
import { fileURLToPath } from "node:url";

const root = fileURLToPath(new URL("../", import.meta.url));
export const ITERATIONS = 10;
// Repetitions spread across shards, so a whole-file selection stays inside
// the Playwright lane's 45-minute job timeout. Playwright shards at file ×
// repetition granularity (workers: 1, no fullyParallel). The regular smoke
// lane runs ~150 journeys per shard in ~11 minutes, about 4.5 s each, so five
// journeys × ten repetitions is a few minutes of test time per shard; it never
// uses more than ten runners, so neither does this one. Past ten shards each
// one grows instead, and the timeout minus dependency setup and the E2E build
// (~10 minutes, more when apt retries) leaves room for about 250 journey-runs
// (~19 minutes) per shard, so a selection beyond that many journeys skips the
// lane with a note rather than timing out.
const JOURNEYS_PER_SHARD = 5;
const MAX_SHARDS = 10;
const MAX_JOURNEY_RUNS_PER_SHARD = 250;
export const playwrightShards = (journeys) =>
  Math.min(MAX_SHARDS, Math.max(1, Math.ceil(journeys / JOURNEYS_PER_SHARD)));
export const MAX_PLAYWRIGHT_JOURNEYS = Math.floor(
  (MAX_JOURNEY_RUNS_PER_SHARD * MAX_SHARDS) / ITERATIONS,
);
// GitHub caps a job's outputs at 1 MiB in total. A selection this large, like
// one past the journey cap, is a sweeping refactor rather than a test change
// worth ten repetitions, so the lane skips rather than overrunning its timeout.
export const MAX_OUTPUT_BYTES = 64 * 1024;
export const runners = ["rust", "tauri", "node", "playwright", "dart"];
// The two cargo workspaces, by runner: the root workspace's crates, and the
// desktop Tauri workspace (desktop/src-tauri and its crates/*), which the
// Desktop Tauri lane (_ci-desktop.yml) builds with its own manifest, system
// dependencies and cache. Each is repeated by its own lane with nextest run
// against that manifest.
export const rustWorkspaces = {
  rust: "Cargo.toml",
  tauri: "desktop/src-tauri/Cargo.toml",
};
const rustLayouts = [
  {
    runner: "rust",
    pattern: /^(crates\/[^/]+)\/(src|tests)\/((?!fixtures\/).*)\.rs$/,
  },
  {
    runner: "tauri",
    pattern:
      /^(desktop\/src-tauri(?:\/crates\/[^/]+)?)\/(src|tests)\/((?!fixtures\/).*)\.rs$/,
  },
];

// A Rust file's place in its workspace: the runner, its package directory,
// whether it is under src or tests, and its path below that without `.rs`.
export function rustFile(file) {
  for (const { runner, pattern } of rustLayouts) {
    const match = pattern.exec(file);
    if (match)
      return {
        runner,
        dir: match[1],
        kind: match[2],
        parts: match[3].split("/"),
      };
  }
  return undefined;
}
// Mirrors desktop/package.json "test": plain helpers run under the TS loader;
// jsdom tests also preload the DOM shim and force-exit past their timers.
export const nodeTestArgs = (file) =>
  file.endsWith(".jsdom-test.mjs")
    ? [
        "--import",
        "./test-jsdom-setup.mjs",
        "--import",
        "./test-loader.mjs",
        "--experimental-strip-types",
        "--test-force-exit",
        "--test",
      ]
    : ["--import", "./test-loader.mjs", "--experimental-strip-types", "--test"];
// Mirrors the mobile test shards in .github/workflows/_ci-clients.yml.
export const dartDefine =
  "--dart-define=BUZZ_PUSH_GATEWAY_URL=https://push.example";
// Mirrors the postgres-ci default-filter in .config/nextest.toml
// (`test(/postgres_tests::/)`, `binary(/^postgres_/)` and
// `test(/(^|::)external_infra[^:]*::/)`): those tests need Postgres, Redis,
// S3 or MinIO, so a selection that reached one would fail every repetition
// without being flaky. Only the root workspace has such tests.
export const infraTest = (binaryName, name) =>
  /postgres_tests::/.test(name) ||
  /^postgres_/.test(binaryName) ||
  /(^|::)external_infra[^:]*::/.test(name);

// The feature graphs the regular lanes test a package in, in order, where
// that is not just its default graph: the Justfile's test-unit runs
// buzz-push-gateway without and with its feature and buzz-feature-flags only
// with LaunchDarkly, and desktop-tauri-test-mesh runs the desktop crate's lib
// tests (`lib`: no other target) under mesh-llm. A test behind
// `#[cfg(feature = …)]` is listed only in its graph, and one listed in several
// can compile differently in each (`#[cfg(not(feature = …))]` arms).
export const rustFeatureGraphs = {
  "buzz-feature-flags": [{ features: ["launchdarkly"] }],
  "buzz-push-gateway": [
    { features: [] },
    { features: ["personal-dev-app-attest"] },
  ],
  "buzz-desktop": [{ features: [] }, { features: ["mesh-llm"], lib: true }],
};

// Mirrors desktop/package.json "test", desktop/playwright.config.ts testDir,
// the two cargo workspaces' packages and mobile's test directory. Paths are
// later split on whitespace in a shell, so admit only plain names. The
// desktop `*.perf.ts` measurements are deliberately not specs here: they time
// the app rather than assert on it, so a slow repetition is not a flake.
export function classify(file) {
  if (!/^[\w./@-]+$/.test(file)) return undefined;
  const rust = rustFile(file);
  if (rust) return rust.runner;
  if (
    /^desktop\/(?:src\/.*\.(?:test|jsdom-test)\.mjs|scripts\/[^/]+\.test\.mjs)$/.test(
      file,
    )
  )
    return "node";
  if (/^desktop\/tests\/e2e\/[^/]+\.spec\.ts$/.test(file)) return "playwright";
  if (/^mobile\/test\/.*_test\.dart$/.test(file)) return "dart";
  return undefined;
}

// Line ranges of a -U0 diff: the new-side lines each hunk writes and the
// old-side lines it removes (empty, end < start, for a pure insertion). A pure
// deletion writes no lines and attaches to the line before it (0 at the top of
// the file); `removed` keeps what it took out, since that line cannot tell.
export function hunkRanges(diff) {
  return [
    ...diff.matchAll(/^@@ -(\d+)(?:,(\d+))? \+(\d+)(?:,(\d+))? @@/gm),
  ].map(([, oldStart, oldCount = "1", start, count = "1"]) => ({
    start: Number(start),
    end: Number(start) + Math.max(Number(count), 1) - 1,
    removed: {
      start: Number(oldStart),
      end: Number(oldStart) + Number(oldCount) - 1,
    },
  }));
}

// Members that keep a `test`/`it` chain a test declaration. Anything else on
// the chain is a group (`describe`), a hook (`test.beforeEach`), configuration
// (`test.use`) or a harness object that happens to be named `test`.
const declaring = [
  "only",
  "skip",
  "fixme",
  "fail",
  "fails",
  "todo",
  "concurrent",
  "sequential",
  "each",
  "for",
  "runIf",
  "skipIf",
];

// The desktop app's TypeScript parses the specs; load it only for a selection
// that needs it, so Rust-only runs work without desktop dependencies.
let typescript;
const ts = () =>
  (typescript ??= createRequire(
    new URL("../desktop/package.json", import.meta.url),
  )("typescript"));

// Line spans of every test declaration in a TypeScript spec, from its first
// line to its closing parenthesis.
export function testCalls(source, file) {
  const t = ts();
  const sourceFile = t.createSourceFile(
    file,
    source,
    t.ScriptTarget.Latest,
    true,
  );
  const lineAt = (position) =>
    sourceFile.getLineAndCharacterOfPosition(position).line + 1;
  const names = (callee) =>
    t.isIdentifier(callee)
      ? [callee.text]
      : t.isPropertyAccessExpression(callee)
        ? names(callee.expression)?.concat(callee.name.text)
        : t.isCallExpression(callee)
          ? names(callee.expression)
          : undefined;
  const calls = [];
  const visit = (node) => {
    const chain = t.isCallExpression(node) ? names(node.expression) : undefined;
    if (
      chain &&
      ["test", "it"].includes(chain[0]) &&
      chain.slice(1).every((member) => declaring.includes(member))
    ) {
      calls.push({
        start: lineAt(node.getStart(sourceFile)),
        end: lineAt(node.end - 1),
      });
      return;
    }
    t.forEachChild(node, visit);
  };
  visit(sourceFile);
  return calls;
}

// Rust source with every comment and literal blanked to spaces, newlines kept,
// so attribute and brace scanning never see a `{` or `#[` inside a string,
// char literal or comment. Lifetimes (`'a`) are kept; char literals are not.
export function blankRust(source) {
  let out = "";
  let i = 0;
  const blank = (end) => {
    for (; i < end && i < source.length; i += 1)
      out += source[i] === "\n" ? "\n" : " ";
  };
  const closingQuote = (from, raw = false, hashes = "") => {
    for (let j = from; j < source.length; j += 1) {
      if (source[j] === "\\" && !raw) j += 1;
      else if (source[j] === '"' && source.startsWith(hashes, j + 1))
        return j + 1 + hashes.length;
    }
    return source.length;
  };
  const rawPrefix = /[bc]?r(#*)"/y;
  while (i < source.length) {
    const rest = source.slice(i, i + 2);
    rawPrefix.lastIndex = i;
    const raw = rawPrefix.exec(source);
    if (rest.startsWith("//")) {
      const end = source.indexOf("\n", i);
      blank(end === -1 ? source.length : end);
    } else if (rest.startsWith("/*")) {
      let depth = 0;
      let j = i;
      do {
        if (source.startsWith("/*", j)) {
          depth += 1;
          j += 2;
        } else if (source.startsWith("*/", j)) {
          depth -= 1;
          j += 2;
        } else j += 1;
      } while (depth > 0 && j < source.length);
      blank(j);
    } else if (raw) {
      blank(closingQuote(i + raw[0].length, true, raw[1]));
    } else if (/^[bc]?"/.test(rest)) {
      blank(closingQuote(i + rest.indexOf('"') + 1));
    } else if (source[i] === "'") {
      const width = source.codePointAt(i + 1) > 0xffff ? 2 : 1;
      if (source[i + 1] === "\\") blank(source.indexOf("'", i + 2) + 1);
      else if (source[i + 1 + width] === "'") blank(i + width + 2);
      else {
        out += source[i];
        i += 1;
      }
    } else {
      out += source[i];
      i += 1;
    }
  }
  return out;
}

const matching = (text, open, close, from) => {
  let depth = 0;
  for (let j = from; j < text.length; j += 1) {
    if (text[j] === open) depth += 1;
    else if (text[j] === close) {
      depth -= 1;
      if (depth === 0) return j;
    }
  }
  return text.length - 1;
};

// Every test function in a Rust file: a `fn` whose attribute block has an
// attribute path ending in `test` (`#[test]`, `#[tokio::test]`, …), spanning
// the first attribute line to the body's closing brace. `name` is the path
// within the file through its inline modules (`tests::plain`), so a namesake
// in another module stays a different test. `ignored` marks `#[ignore]`,
// which nextest skips by default.
export function rustTests(source) {
  const text = blankRust(source);
  const modules = [];
  const inline = /\bmod\s+(\w+)\s*\{/g;
  for (let m = inline.exec(text); m; m = inline.exec(text)) {
    const open = m.index + m[0].length - 1;
    modules.push({ name: m[1], open, close: matching(text, "{", "}", open) });
  }
  const lineStarts = [0];
  for (let i = 0; i < text.length; i += 1)
    if (text[i] === "\n") lineStarts.push(i + 1);
  const lineAt = (offset) => {
    let low = 0;
    let high = lineStarts.length - 1;
    while (low < high) {
      const mid = Math.ceil((low + high) / 2);
      if (lineStarts[mid] <= offset) low = mid;
      else high = mid - 1;
    }
    return low + 1;
  };
  const tests = [];
  const attribute = /#\[/g;
  for (let match = attribute.exec(text); match; match = attribute.exec(text)) {
    const paths = [];
    let j = match.index;
    while (text.startsWith("#[", j)) {
      const close = matching(text, "[", "]", j + 1);
      paths.push(/^\s*([\w:]+)/.exec(text.slice(j + 2, close))?.[1] ?? "");
      j = close + 1;
      while (/\s/.test(text[j] ?? "")) j += 1;
    }
    attribute.lastIndex = j;
    if (!paths.some((path) => /^(?:\w+::)*test$/.test(path))) continue;
    const fn =
      /^(?:pub(?:\([\w\s:]*\))?\s+)?(?:async\s+)?(?:unsafe\s+)?fn\s+(\w+)/.exec(
        text.slice(j),
      );
    if (!fn) continue;
    const open = text.indexOf("{", j + fn[0].length);
    if (open === -1) continue;
    const path = modules
      .filter((mod) => mod.open < match.index && match.index < mod.close)
      .map((mod) => mod.name);
    tests.push({
      name: [...path, fn[1]].join("::"),
      start: lineAt(match.index),
      end: lineAt(matching(text, "{", "}", open)),
      ignored: paths.includes("ignore"),
    });
  }
  return tests;
}

// Bodies of the listed tests: their calls, keyed by the line the runner
// listed. Runners only report a call-site line, which may sit inside the call,
// so each listed line is owned by the enclosing test call; a listed line
// outside every call leaves its lines to the whole-file rule.
export function testBodies(calls, lines) {
  const bodies = new Map();
  for (const line of [...lines].sort((a, b) => a - b)) {
    const call = calls.find(({ start, end }) => start <= line && line <= end);
    if (call && !bodies.has(call)) bodies.set(call, { ...call, line });
  }
  return [...bodies.values()].sort((a, b) => a.start - b.start);
}

// The calls owning every line of a range, or undefined once a line lies
// outside them all.
function owners(calls, { start, end }) {
  const found = [];
  for (let line = start; line <= end; ) {
    const call = calls.find((c) => c.start <= line && line <= c.end);
    if (!call) return undefined;
    found.push(call);
    line = call.end + 1;
  }
  return found;
}

// A hunk whose every line lies in a test body selects those tests. A changed
// line outside every body (imports, helpers, hooks or constants before,
// between or after tests, or the code under test in a Rust module) can change
// every test, so it, a new file, or a file with no listed tests selects the
// whole file. Removed lines are judged by the old file's calls (`previous`): a
// deletion attaches to its preceding line, which belongs to the test before a
// removed hook, not to the hook.
export function selectTests(
  ranges,
  bodies,
  { added = false, previous = [] } = {},
) {
  if (added) return { whole: true, reason: "new file" };
  if (bodies.length === 0) return { whole: true, reason: "no tests listed" };
  const selected = new Set();
  for (const { start, end, removed } of ranges) {
    if (!owners(previous, removed))
      return { whole: true, reason: "shared code removed" };
    const current = owners(bodies, { start, end });
    if (!current) return { whole: true, reason: "shared code changed" };
    for (const body of current) selected.add(body.line);
  }
  return { lines: [...selected].sort((a, b) => a - b) };
}

// Rust tests a diff selects: hunks inside a test body select that test, any
// other hunk selects every test in the file (the module's code under test
// lives beside them) and, for an integration-test root, every test of the
// file modules it declares (`included`: they are compiled into the same
// binary, so a root with no test of its own still selects them). `#[ignore]`d
// tests own their lines but never run. A file with no tests at all selects
// nothing of its own; `select` decides what that means by where it lives.
export function selectRust(
  diff,
  source,
  { added = false, previous = "", included = [] } = {},
) {
  const tests = rustTests(source);
  if (tests.length === 0 && included.length === 0) return undefined;
  const selection =
    tests.length === 0
      ? { whole: true, reason: added ? "new file" : "shared code changed" }
      : selectTests(
          hunkRanges(diff),
          tests.map((test) => ({ ...test, line: test.start })),
          { added, previous: added ? [] : rustTests(previous) },
        );
  const chosen = selection.whole
    ? [...tests, ...included]
    : tests.filter((test) => selection.lines.includes(test.start));
  return {
    ...selection,
    names: chosen.filter((test) => !test.ignored).map((test) => test.name),
  };
}

const run = (command, args, options = {}) => {
  const result = spawnSync(command, args, {
    cwd: root,
    encoding: "utf8",
    maxBuffer: 64 * 1024 * 1024,
    ...options,
  });
  if (result.error) throw result.error;
  return result;
};
const git = (...args) => {
  const result = run("git", args);
  if (result.status !== 0) throw new Error(`git ${args[0]}: ${result.stderr}`);
  return result.stdout;
};
const summarize = (text) => {
  if (process.env.GITHUB_STEP_SUMMARY)
    appendFileSync(process.env.GITHUB_STEP_SUMMARY, text);
  else process.stdout.write(text);
};
const cell = (value) => String(value).replaceAll("|", "\\|");

const crateDir = (file) => `${root}${rustFile(file).dir}`;

// The `[package]` name of the package owning a Rust file.
export function cratePackage(file) {
  const manifest = readFileSync(`${crateDir(file)}/Cargo.toml`, "utf8");
  return /^\[package\][\s\S]*?^name\s*=\s*"([^"]+)"/m.exec(manifest)[1];
}

// Cargo target flags building only the test binaries a file can be part of,
// one flag per entry so rustGroups can merge files' flags without splitting
// `--test <name>`: src/** is the lib (when the crate has one) or a bin,
// tests/<stem>.rs and tests/<dir>/*.rs (with a main.rs) the integration test
// of that name. A file placed by a custom `[[test]]` path builds every target
// instead.
export function rustTarget(file) {
  const { kind, parts } = rustFile(file);
  if (kind === "src") {
    const lib =
      existsSync(`${crateDir(file)}/src/lib.rs`) ||
      /^\[lib\]/m.test(readFileSync(`${crateDir(file)}/Cargo.toml`, "utf8"));
    return lib ? [["--lib"], ["--bins"]] : [["--bins"]];
  }
  if (parts.length === 1) return [["--test", parts[0]]];
  return existsSync(`${crateDir(file)}/tests/${parts[0]}/main.rs`)
    ? [["--test", parts[0]]]
    : [];
}

// The module path a file's items carry in their binary under cargo's layout
// conventions: src/a/b.rs is a::b, a mod.rs/main.rs/lib.rs names its directory
// or the root, src/bin/x.rs and tests/<target>.rs are roots, tests/<target>/x.rs
// is x. A `#[path]` attribute can break this; repeatRust then falls back to the
// in-file path alone.
export function rustModulePath(file) {
  const { kind, parts: below } = rustFile(file);
  const parts = [...below];
  if (kind === "tests") parts.shift();
  else if (parts[0] === "bin") parts.splice(0, 2);
  if (["mod", "main"].includes(parts.at(-1)) || parts.join() === "lib")
    parts.pop();
  return parts;
}

// Whether an integration-test target (tests/<stem>.rs or tests/<dir>/main.rs)
// compiles a helper file under tests/ into its binary: a main.rs owns its
// directory, and any target's source declares `mod <first>;` for the
// helper's first path component under tests/ (a crate root resolves
// `mod common;` to tests/common/mod.rs or tests/common.rs), or a `#[path]`
// attribute names the helper's file or the mod.rs of a directory holding it.
// Declarations are read with comments and strings blanked; `#[path]` strings
// are read raw, since blanking would erase them.
export function rustIncludes(target, source, helper) {
  const targetDir = path.dirname(target);
  const [first] = rustFile(helper).parts;
  if (path.basename(target) === "main.rs" && helper.startsWith(`${targetDir}/`))
    return true;
  if (new RegExp(`\\bmod\\s+${first}\\s*;`).test(blankRust(source)))
    return true;
  for (const [, named] of source.matchAll(/#\[\s*path\s*=\s*"([^"]+)"\s*\]/g)) {
    const resolved = path.normalize(path.join(targetDir, named));
    if (resolved === helper) return true;
    if (
      path.basename(resolved) === "mod.rs" &&
      helper.startsWith(`${path.dirname(resolved)}/`)
    )
      return true;
  }
  return false;
}

// The integration-test targets of a helper's package that include it, as
// files. A helper no target is found to include (a layout rustIncludes does
// not read) falls back to every target, so a helper change is never left
// unrepeated by the scan.
export function rustImporters(helper) {
  const { dir } = rustFile(helper);
  const tests = `${dir}/tests`;
  const targets = readdirSync(`${root}${tests}`, { withFileTypes: true })
    .flatMap((entry) =>
      entry.isFile() && entry.name.endsWith(".rs")
        ? [`${tests}/${entry.name}`]
        : entry.isDirectory() &&
            existsSync(`${root}${tests}/${entry.name}/main.rs`)
          ? [`${tests}/${entry.name}/main.rs`]
          : [],
    )
    .filter((target) => target !== helper)
    .sort();
  const importers = targets.filter((target) =>
    rustIncludes(target, readFileSync(`${root}${target}`, "utf8"), helper),
  );
  return {
    importers: importers.length ? importers : targets,
    scanned: importers.length > 0,
  };
}

// Whether a tests/** file is the root of an integration-test binary
// (tests/<stem>.rs or tests/<dir>/main.rs) rather than a module of one.
export function rustTargetRoot(file) {
  const place = rustFile(file);
  if (place?.kind !== "tests") return false;
  const { parts } = place;
  return parts.length === 1 || (parts.length === 2 && parts[1] === "main");
}

// The file modules a Rust file declares, as cargo compiles them into its
// binary: `#[path = "…"] mod x;` names a file relative to the declaring
// file's directory; a plain `mod x;` is x.rs or x/mod.rs under the module's
// directory, the file's own for a root (lib.rs, main.rs, mod.rs, a
// tests/<stem>.rs) and <dir>/<stem>/ otherwise. Declarations are read with
// comments and strings blanked; `#[path]` strings raw, since blanking erases
// them. A module no file backs is left out.
export function rustModules(file, source) {
  const dir = path.dirname(file);
  const stem = path.basename(file, ".rs");
  const owned =
    ["lib", "main", "mod"].includes(stem) || rustTargetRoot(file)
      ? dir
      : path.join(dir, stem);
  const pathed = new Map();
  for (const [, named, name] of source.matchAll(
    /#\[\s*path\s*=\s*"([^"]+)"\s*\]\s*(?:#\[[^\]]*\]\s*)*(?:pub(?:\([^)]*\))?\s+)?mod\s+(\w+)\s*;/g,
  ))
    pathed.set(name, path.normalize(path.join(dir, named)));
  const modules = [];
  for (const [, name] of blankRust(source).matchAll(
    /\b(?:pub(?:\([^)]*\))?\s+)?mod\s+(\w+)\s*;/g,
  )) {
    const found = (
      pathed.has(name)
        ? [pathed.get(name)]
        : [`${owned}/${name}.rs`, `${owned}/${name}/mod.rs`]
    ).find((candidate) => existsSync(`${root}${candidate}`));
    if (found) modules.push({ name, file: found });
  }
  return modules;
}

// The tests a file compiles in from the file modules it declares,
// transitively, named by their module path from it
// (`pool_lifecycle::tests::name` for `#[path = "../src/pool_lifecycle.rs"]
// mod pool_lifecycle;`), which is how nextest lists them for the binary. An
// integration-test root with no test function of its own (a wrapper around a
// source module, as buzz-acp's pool_lifecycle_state) has only these.
export function rustIncludedTests(file, source, seen = new Set([file])) {
  return rustModules(file, source).flatMap(({ name, file: module }) => {
    if (seen.has(module)) return [];
    seen.add(module);
    const text = readFileSync(`${root}${module}`, "utf8");
    return [...rustTests(text), ...rustIncludedTests(module, text, seen)].map(
      (test) => ({ ...test, name: `${name}::${test.name}` }),
    );
  });
}

// Smoke journeys per spec file as Playwright lists them, each line once (a
// looped declaration is listed once per iteration). Integration-only specs
// list nothing for the smoke project, which Playwright reports as its "No
// tests found" error, so a selection of them stays empty.
export function listPlaywright(files) {
  const result = run(`${root}bin/pnpm`, [
    "-C",
    "desktop",
    "--silent",
    "exec",
    "playwright",
    "test",
    "--project=smoke",
    "--list",
    "--reporter=json",
    ...files,
  ]);
  const report = JSON.parse(result.stdout);
  const empty =
    report.errors.length === 1 &&
    /No tests found/.test(report.errors[0].message);
  if ((result.status !== 0 || report.errors.length) && !empty)
    throw new Error(
      `playwright --list: ${report.errors.map((error) => error.message).join("\n") || result.stderr}`,
    );
  const specs = new Map(files.map((file) => [file, new Set()]));
  const collect = (suites) => {
    for (const suite of suites) {
      for (const spec of suite.specs ?? [])
        if (spec.tests?.length)
          specs.get(`desktop/tests/e2e/${spec.file}`)?.add(spec.line);
      collect(suite.suites ?? []);
    }
  };
  collect(empty ? [] : report.suites);
  return new Map([...specs].map(([file, lines]) => [file, [...lines]]));
}

function select(base = "HEAD^1", head = "HEAD") {
  const changed = git(
    "diff",
    "--name-status",
    "--no-renames",
    "-z",
    "--diff-filter=AM",
    base,
    head,
    "--",
  ).split("\0");
  const files = Object.fromEntries(runners.map((runner) => [runner, []]));
  const added = new Set();
  for (let i = 0; i + 1 < changed.length; i += 2) {
    const [status, file] = [changed[i], changed[i + 1]];
    const runner = classify(file);
    if (runner) files[runner].push(file);
    if (status === "A") added.add(file);
  }
  const journeys = files.playwright.length
    ? listPlaywright(files.playwright)
    : new Map();
  const outputs = Object.fromEntries(runners.map((runner) => [runner, []]));
  const rows = [];
  const previous = (file) =>
    added.has(file) ? "" : git("show", `${base}:${file}`);
  const diff = (file) =>
    git("diff", "-U0", "--no-color", base, head, "--", file);
  // The tests an integration-test root compiles in from its file modules;
  // any other file's tests are its own.
  const includedRust = (file, source) =>
    rustTargetRoot(file) ? rustIncludedTests(file, source) : [];
  // Every runnable test of a Rust file, as entries.
  const wholeRust = (file) => {
    const source = readFileSync(`${root}${file}`, "utf8");
    return [...rustTests(source), ...includedRust(file, source)]
      .filter((test) => !test.ignored)
      .map((test) => `${file}::${test.name}`);
  };
  for (const runner of runners) {
    for (const file of files[runner]) {
      let entries = [file];
      let note = "whole file";
      if (rustWorkspaces[runner]) {
        const source = readFileSync(`${root}${file}`, "utf8");
        const selection = selectRust(diff(file), source, {
          added: added.has(file),
          previous: previous(file),
          included: includedRust(file, source),
        });
        if (selection) {
          entries = selection.names.map((name) => `${file}::${name}`);
          note =
            selection.names.length === 0
              ? "excluded (ignored tests only)"
              : selection.whole
                ? `whole file (${selection.reason}): ${selection.names.length} test(s)`
                : `${selection.names.length} changed test(s)`;
        } else if (rustTargetRoot(file)) {
          // An integration-test root with no test of its own and none in
          // the modules it declares: a binary with nothing to repeat.
          rows.push(
            `| ${cell(file)} | ${runner} | integration test with no tests |`,
          );
          continue;
        } else if (rustFile(file).kind === "tests") {
          // A tests/** file that is not a binary's root and has no test
          // functions is a helper module that integration binaries compile
          // in (`mod common;`), so a change to it can change every test of
          // each binary that includes it: select those binaries whole. The
          // ignored and infrastructure exclusions still apply downstream, as
          // for any whole-file selection.
          const { importers, scanned } = rustImporters(file);
          entries = importers.flatMap(wholeRust);
          const names = importers.map(
            (importer) => rustFile(importer).parts[0],
          );
          note =
            importers.length === 0
              ? "helper with no integration tests to repeat"
              : `helper for${scanned ? "" : " (no importer found; every integration test)"}: ${names.join(", ")}: ${entries.length} test(s)`;
        } else {
          // A src/** file without test functions is the code under test, not
          // a test file. This is deliberate: the regular lanes build and run
          // everything that depends on it, and repeating its crate's whole
          // suite for any source change would not be a changed-test check.
          rows.push(`| ${cell(file)} | ${runner} | no test functions |`);
          continue;
        }
      } else if (runner === "playwright") {
        const listed = journeys.get(file) ?? [];
        const source = readFileSync(`${root}${file}`, "utf8");
        const selection = selectTests(
          hunkRanges(diff(file)),
          testBodies(testCalls(source, file), listed),
          {
            added: added.has(file),
            previous: added.has(file) ? [] : testCalls(previous(file), file),
          },
        );
        const lines = selection.whole ? listed : selection.lines;
        entries = lines.map((line) => `${file}:${line}`);
        note =
          lines.length === 0
            ? "excluded (no smoke journeys)"
            : selection.whole
              ? `whole file (${selection.reason}): ${lines.length} journey(s)`
              : `${lines.length} changed journey(s)`;
      }
      outputs[runner].push(...entries);
      rows.push(`| ${cell(file)} | ${runner} | ${cell(note)} |`);
    }
    // A helper and a file that includes it, changed together, select the
    // same tests twice.
    outputs[runner] = [...new Set(outputs[runner])];
    const exceeded =
      Buffer.byteLength(outputs[runner].join(" ")) > MAX_OUTPUT_BYTES
        ? `selection exceeds ${MAX_OUTPUT_BYTES} bytes`
        : runner === "playwright" &&
            outputs[runner].length > MAX_PLAYWRIGHT_JOURNEYS
          ? `selection exceeds ${MAX_PLAYWRIGHT_JOURNEYS} journeys, which ${ITERATIONS} repetitions would not finish in time`
          : undefined;
    if (exceeded) {
      rows.push(
        `| *(${outputs[runner].length} ${runner} entries)* | ${runner} | ${exceeded}; lane skipped |`,
      );
      outputs[runner] = [];
    }
  }
  summarize(
    [
      "## Flake check selection",
      "",
      ...(rows.length
        ? ["| File | Runner | Selection |", "| --- | --- | --- |", ...rows]
        : [
            "No Rust, desktop Tauri, node:test, Playwright or Flutter test files changed.",
          ]),
      "",
      "",
    ].join("\n"),
  );
  const shards = playwrightShards(outputs.playwright.length);
  const text = [
    ...Object.entries(outputs).map(
      ([key, entries]) => `${key}=${entries.join(" ")}\n`,
    ),
    `playwright_shards=${JSON.stringify(Array.from({ length: shards }, (_, i) => i + 1))}\n`,
    `playwright_shard_total=${shards}\n`,
  ].join("");
  if (process.env.GITHUB_OUTPUT)
    appendFileSync(process.env.GITHUB_OUTPUT, text);
  else process.stdout.write(text);
}

const resultsDir = (runner) => {
  const dir = `${root}test-results/flake/${runner}`;
  rmSync(dir, { recursive: true, force: true });
  mkdirSync(dir, { recursive: true });
  return dir;
};

// Test × iteration table for a repeated selection; a run passes only when its
// process exits 0 and every expected test reports. Tests that never ran in any
// iteration are left out, so the table holds only the repeated ones.
function report(title, runs) {
  const columns = runs.map((_, i) => String(i + 1));
  const mark = (status) =>
    ({ passed: "✅", failed: "❌" })[status] ?? (status ? "⏭" : "—");
  const ran = (status) => status === "passed" || status === "failed";
  const tests = [
    ...new Set(runs.flatMap((run) => [...run.statuses.keys()])),
  ].filter((test) => runs.some((run) => ran(run.statuses.get(test))));
  const failures = runs.filter((run) => !run.ok).length;
  summarize(
    [
      `## ${title}: ${failures} of ${runs.length} iterations failed`,
      "",
      `| Test | ${columns.join(" | ")} | Failed |`,
      `| --- | ${columns.map(() => ":-:").join(" | ")} | ---: |`,
      `| **Run** | ${runs.map((run) => (run.ok ? "✅" : "❌")).join(" | ")} | ${failures}/${runs.length} |`,
      ...tests.sort().map((test) => {
        const statuses = runs.map((run) => run.statuses.get(test));
        return `| ${cell(test)} | ${statuses.map(mark).join(" | ")} | ${statuses.filter((status) => status === "failed").length}/${runs.length} |`;
      }),
      "",
      "",
    ].join("\n"),
  );
  process.exitCode = failures === 0 ? 0 : 1;
}

const progress = (i, iterations, ok) =>
  console.log(
    `flake-check: iteration ${i}/${iterations} ${ok ? "passed" : "FAILED"}`,
  );

// Per-test statuses of a JUnit report, keyed `<classname> <name>`.
export function parseJunit(xml) {
  const decode = (text) =>
    text.replace(
      /&(quot|apos|lt|gt|amp);/g,
      (_, entity) =>
        ({ quot: '"', apos: "'", lt: "<", gt: ">", amp: "&" })[entity],
    );
  const statuses = new Map();
  for (const chunk of xml.split("<testcase").slice(1)) {
    const attributes = chunk.slice(0, chunk.indexOf(">"));
    const body = chunk.split("</testsuite>")[0];
    const name = decode(/\bname="([^"]*)"/.exec(attributes)[1]);
    const classname = decode(
      /\bclassname="([^"]*)"/.exec(attributes)?.[1] ?? "",
    );
    statuses.set(
      `${classname} ${name}`,
      /<(?:failure|error)[\s/>]/.test(body)
        ? "failed"
        : /<skipped[\s/>]/.test(body)
          ? "skipped"
          : "passed",
    );
  }
  return statuses;
}

// The listed tests an entry stands for: a test the file's module path predicts
// exactly when nextest lists one, otherwise every test whose name ends in the
// in-file path (a `#[path]` module breaks the prediction; a namesake in another
// module is then repeated too rather than nothing). `unlisted` keeps the
// entries no candidate stands for.
export function chooseRust(tests, candidates) {
  const chosen = new Map();
  const unlisted = [];
  for (const test of tests) {
    const { path, exact } = test;
    const suffix = candidates.filter(
      ({ name }) => name === path || name.endsWith(`::${path}`),
    );
    const predicted = suffix.filter(({ name }) => name === exact);
    if (suffix.length === 0) unlisted.push(test);
    for (const test of predicted.length ? predicted : suffix)
      chosen.set(`${test.binary} ${test.name}`, test);
  }
  return { chosen: [...chosen.values()], unlisted };
}

// The repeat's cargo invocation per package: `-p <crate>` plus the target
// flags of its selected files, each whole flag once (`--test codec --test
// connection`; merging tokens instead gave `--test codec connection`, which
// nextest reads as a test-name filter that matches nothing), the feature
// graphs to list them in with each graph's full arguments, and the tests to
// list in those targets. Every entry must belong to one workspace, whose
// runner is returned with the groups.
export function rustGroups(entries) {
  const groups = new Map();
  const workspaces = new Set();
  for (const entry of entries) {
    const [, file, test] = /^([^:]+\.rs)::((?:\w+::)*\w+)$/.exec(entry) ?? [];
    const place = file && rustFile(file);
    if (!place)
      throw new Error(`rust entry must be <file>::<module::test>: ${entry}`);
    workspaces.add(place.runner);
    const crate = cratePackage(file);
    const group = groups.get(crate) ?? { targets: new Map(), tests: [] };
    const targets = rustTarget(file);
    if (targets.length === 0) group.every = true;
    for (const target of targets) group.targets.set(target.join(" "), target);
    group.tests.push({
      entry,
      path: test,
      exact: [...rustModulePath(file), test].join("::"),
    });
    groups.set(crate, group);
  }
  if (workspaces.size > 1)
    throw new Error(
      `rust entries span workspaces: ${[...workspaces].join(", ")}`,
    );
  for (const [crate, group] of groups) {
    group.args = [
      "-p",
      crate,
      ...(group.every ? [] : [...group.targets.values()].flat()),
    ];
    // A lib-only graph applies only where the selection can reach the lib.
    group.graphs = (rustFeatureGraphs[crate] ?? [{ features: [] }])
      .filter(({ lib }) => !lib || group.every || group.targets.has("--lib"))
      .map(({ features, lib }) => ({
        features,
        args: [
          ...(lib ? ["-p", crate, "--lib"] : group.args),
          ...(features.length ? ["--features", features.join(",")] : []),
        ],
      }));
  }
  return { groups, runner: [...workspaces][0] };
}

// nextest has no repeat flag, so run the selection once per iteration under
// the `flake` profile (the workspace's .config/nextest.toml: no retries, JUnit
// per run), one cargo invocation per package and feature graph so each builds
// only the targets its files can be in. Each package's targets are listed
// whole in each of its feature graphs (nextest reports an ignored test as a
// filter mismatch whatever `-E` says, so a filtered listing could not tell an
// ignored entry from an unlisted one), and every entry is narrowed with
// chooseRust to exact (binary, test) pairs in every graph listing it, as the
// regular lanes run it in each: a `#[cfg(feature = …)]` arm inside a test
// the default graph lists compiles only in its feature's graph. Statuses are
// keyed per graph, so one graph's pass cannot stand for another's failure.
// An entry standing only for ignored or infrastructure-backed tests is
// excluded; one no graph lists (a test behind a cfg the regular lanes do not
// build here) fails the lane rather than passing unrepeated. The Tauri
// workspace is addressed by its manifest; the root one is cargo's default.
function repeatRust(entries, iterations) {
  const { groups, runner } = rustGroups(entries);
  const title =
    runner === "rust" ? "Flake check (Rust)" : "Flake check (Desktop Tauri)";
  const manifest = rustWorkspaces[runner];
  const cargoArgs = runner === "rust" ? [] : ["--manifest-path", manifest];
  const invocations = [];
  const unlisted = [];
  for (const [crate, group] of groups) {
    const missing = new Set(group.tests);
    for (const { features, args } of group.graphs) {
      const listed = run("cargo", [
        "nextest",
        "list",
        ...cargoArgs,
        ...args,
        "--message-format",
        "json",
      ]);
      if (listed.status !== 0)
        throw new Error(`cargo nextest list: ${listed.stderr}`);
      const candidates = Object.entries(
        JSON.parse(listed.stdout)["rust-suites"],
      ).flatMap(([binary, suite]) =>
        Object.entries(suite.testcases).map(([name, test]) => ({
          binary,
          name,
          runnable: !test.ignored && !infraTest(suite["binary-name"], name),
        })),
      );
      const choice = chooseRust(group.tests, candidates);
      for (const test of group.tests)
        if (!choice.unlisted.includes(test)) missing.delete(test);
      const chosen = choice.chosen.filter((test) => test.runnable);
      if (chosen.length)
        invocations.push({
          label: [crate, ...features].join("+"),
          tag: features.length ? ` [${features.join(",")}]` : "",
          args,
          chosen,
          filterset: chosen
            .map(
              ({ binary, name }) => `(binary_id(=${binary}) & test(=${name}))`,
            )
            .join(" | "),
        });
    }
    unlisted.push(...[...missing].map((test) => test.entry));
  }
  if (unlisted.length) {
    summarize(
      [
        `## ${title}: ${unlisted.length} selected test(s) not listed`,
        "",
        "No feature graph the regular lanes test these packages in lists them on this runner, so they were not repeated:",
        "",
        ...unlisted.map((entry) => `- \`${entry}\``),
        "",
        "",
      ].join("\n"),
    );
    process.exitCode = 1;
  }
  const expected = invocations.flatMap((invocation) =>
    invocation.chosen.map(
      ({ binary, name }) => `${binary} ${name}${invocation.tag}`,
    ),
  );
  if (expected.length === 0) {
    if (!unlisted.length)
      summarize(
        `## ${title}: nothing to repeat\n\nEvery selected test is ignored or infrastructure-backed.\n\n`,
      );
    return;
  }
  // nextest keeps its store under the workspace root's target/nextest, not
  // CARGO_TARGET_DIR; the `flake` profile writes its JUnit report there.
  const junit = path.join(
    root,
    path.dirname(manifest),
    "target/nextest/flake/junit.xml",
  );
  const dir = resultsDir(runner);
  const runs = [];
  for (let i = 1; i <= iterations; i += 1) {
    const statuses = new Map();
    let ok = true;
    for (const { label, tag, args, filterset } of invocations) {
      rmSync(junit, { force: true });
      const result = run(
        "cargo",
        [
          "nextest",
          "run",
          ...cargoArgs,
          "--profile",
          "flake",
          ...args,
          "-E",
          filterset,
        ],
        { stdio: "inherit" },
      );
      ok &&= result.status === 0;
      try {
        for (const [test, status] of parseJunit(readFileSync(junit, "utf8")))
          statuses.set(`${test}${tag}`, status);
        copyFileSync(junit, `${dir}/iter-${i}-${label}.xml`);
      } catch {
        ok = false;
      }
    }
    ok &&= expected.every((test) => statuses.get(test) === "passed");
    runs.push({ ok, statuses });
    progress(i, iterations, ok);
  }
  report(title, runs);
  if (unlisted.length) process.exitCode = 1;
}

// node:test has no repeat flag either. Each file runs alone, so a failure is
// attributed to its file; the regular lane's cross-file concurrency is not
// reproduced here.
function repeatNode(files, iterations) {
  const dir = resultsDir("node");
  const runs = [];
  for (let i = 1; i <= iterations; i += 1) {
    const statuses = new Map();
    for (const file of files) {
      const path = file.replace(/^desktop\//, "");
      const result = run(process.execPath, [...nodeTestArgs(path), path], {
        cwd: `${root}desktop`,
      });
      process.stdout.write(result.stdout);
      process.stderr.write(result.stderr);
      appendFileSync(
        `${dir}/iter-${i}.log`,
        `# ${file}\n${result.stdout}${result.stderr}\n`,
      );
      statuses.set(file, result.status === 0 ? "passed" : "failed");
    }
    const ok = [...statuses.values()].every((status) => status === "passed");
    runs.push({ ok, statuses });
    progress(i, iterations, ok);
  }
  report("Flake check (node:test)", runs);
}

// Per-test statuses of a `dart test` JSON report (one event per line), keyed
// `<file> › <name>`. Hidden entries are the runner's own file-loading steps.
export function parseDartReport(text, mobileRoot = `${root}mobile/`) {
  const started = new Map();
  const statuses = new Map();
  for (const line of text.split("\n")) {
    let event;
    try {
      event = JSON.parse(line);
    } catch {
      continue;
    }
    if (event.type === "testStart") started.set(event.test.id, event.test);
    if (event.type !== "testDone" || event.hidden) continue;
    const test = started.get(event.testID);
    if (!test) continue;
    const file = fileURLToPath(test.url ?? "file:///").replace(mobileRoot, "");
    statuses.set(
      `${file} › ${test.name}`,
      event.skipped
        ? "skipped"
        : event.result === "success"
          ? "passed"
          : "failed",
    );
  }
  return statuses;
}

// `flutter test` has no repeat flag; each iteration recompiles only the
// selected files, as the sharded mobile lane does.
function repeatDart(files, iterations) {
  const dir = resultsDir("dart");
  const runs = [];
  // The Justfile unsets these so a worktree's git context never reaches flutter.
  const { GIT_DIR, GIT_WORK_TREE, ...env } = process.env;
  for (let i = 1; i <= iterations; i += 1) {
    const reportFile = `${dir}/iter-${i}.json`;
    const result = run(
      "flutter",
      [
        "test",
        dartDefine,
        `--file-reporter=json:${reportFile}`,
        ...files.map((file) => file.replace(/^mobile\//, "")),
      ],
      { cwd: `${root}mobile`, env, stdio: "inherit" },
    );
    let statuses = new Map();
    try {
      statuses = parseDartReport(readFileSync(reportFile, "utf8"));
    } catch {
      statuses = new Map();
    }
    const ok = result.status === 0 && statuses.size > 0;
    runs.push({ ok, statuses });
    progress(i, iterations, ok);
  }
  report("Flake check (Flutter)", runs);
}

// Playwright's JSON report lists a `--repeat-each` journey once per
// repetition, in order; `unexpected` is a failed repetition since the lane
// runs with --retries=0.
export function playwrightRepetitions(report) {
  const repetitions = new Map();
  const collect = (suites) => {
    for (const suite of suites) {
      for (const spec of suite.specs ?? []) {
        const key = `${spec.file}:${spec.line} › ${spec.title}`;
        const statuses = repetitions.get(key) ?? [];
        for (const test of spec.tests ?? [])
          statuses.push(
            { expected: "passed", unexpected: "failed", flaky: "failed" }[
              test.status
            ] ?? "skipped",
          );
        repetitions.set(key, statuses);
      }
      collect(suite.suites ?? []);
    }
  };
  collect(report.suites ?? []);
  return repetitions;
}

function playwrightSummary(reportPath, title) {
  const repetitions = playwrightRepetitions(
    JSON.parse(readFileSync(reportPath, "utf8")),
  );
  const iterations = Math.max(
    0,
    ...[...repetitions.values()].map((s) => s.length),
  );
  const runs = Array.from({ length: iterations }, (_, i) => ({
    ok: [...repetitions.values()].every((statuses) => statuses[i] !== "failed"),
    statuses: new Map(
      [...repetitions].map(([test, statuses]) => [test, statuses[i]]),
    ),
  }));
  report(title, runs);
  // Playwright already failed its own step; the summary only reports.
  process.exitCode = 0;
}

function main() {
  const [command, ...args] = process.argv.slice(2);
  if (command === "select" && args.length <= 2) return select(...args);
  if (command === "playwright-summary" && args.length === 2)
    return playwrightSummary(...args);
  const repeat = {
    rust: repeatRust,
    tauri: repeatRust,
    node: repeatNode,
    dart: repeatDart,
  }[command];
  if (repeat) {
    const iterations = Number(
      args
        .find((argument) => argument.startsWith("--iterations="))
        ?.slice("--iterations=".length) ?? ITERATIONS,
    );
    const entries = args.filter((argument) => !argument.startsWith("--"));
    if (Number.isInteger(iterations) && iterations > 0 && entries.length > 0)
      return repeat(entries, iterations);
  }
  throw new Error(
    "usage: flake-check select [base] [head] | rust|tauri|node|dart [--iterations=N] <entry…> | playwright-summary <report.json> <title>",
  );
}

if (process.argv[1] === fileURLToPath(import.meta.url)) main();
