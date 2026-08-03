import assert from "node:assert/strict";
import { mkdir, mkdtemp, rename, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { test } from "node:test";
import { pathToFileURL } from "node:url";
import {
  assertPiResourceBudget,
  assertPiResourceSnapshotsEqual,
} from "../dist/index.js";
import { testConfig } from "./helpers.mjs";

async function fixture(prefix) {
  const root = await mkdtemp(join(tmpdir(), prefix));
  const cwd = join(root, "workspace");
  const agentDir = join(root, "agent");
  await mkdir(cwd, { recursive: true });
  await mkdir(agentDir, { recursive: true });
  return { root, cwd, agentDir };
}

async function isolatedHome(root, operation) {
  const previous = process.env.HOME;
  process.env.HOME = root;
  try {
    return await operation();
  } finally {
    if (previous === undefined) delete process.env.HOME;
    else process.env.HOME = previous;
  }
}

function scan(cwd, agentDir, overrides = {}) {
  return assertPiResourceBudget({
    cwd,
    agentDir,
    projectTrusted: true,
    config: testConfig(overrides),
  });
}

test("context fallback checks the first real file after a directory candidate", async () => {
  const { root, cwd, agentDir } = await fixture("buzz-pi-context-fallback-");
  await mkdir(join(cwd, "AGENTS.md"));
  await writeFile(join(cwd, "CLAUDE.md"), "x".repeat(65));
  await isolatedHome(root, async () => {
    assert.throws(
      () => scan(cwd, agentDir, { maxResourceFileBytes: 64 }),
      /BUZZ_PI_RESOURCE_LIMIT: context file exceeds the per-file limit/,
    );
  });
});

test("global prompt ignore manifests are bounded before Pi discovery", async () => {
  const { root, cwd, agentDir } = await fixture("buzz-pi-prompt-ignore-");
  await mkdir(join(agentDir, "prompts"));
  await writeFile(join(agentDir, "prompts", ".ignore"), "x".repeat(513));
  await isolatedHome(root, async () => {
    assert.throws(
      () => scan(cwd, agentDir, { maxResourceFileBytes: 512 }),
      /BUZZ_PI_RESOURCE_LIMIT: prompt ignore manifest exceeds/,
    );
  });
});

test("package prompt/theme trees include nested resources and nested ignore manifests", async () => {
  const { root, cwd, agentDir } = await fixture("buzz-pi-package-resource-");
  const pkg = join(agentDir, "fixture-package");
  const prompt = join(pkg, "prompts", "nested", "review.md");
  const theme = join(pkg, "themes", "nested", "dark.json");
  const nestedIgnore = join(pkg, "prompts", "nested", ".fdignore");
  await mkdir(join(pkg, "prompts", "nested"), { recursive: true });
  await mkdir(join(pkg, "themes", "nested"), { recursive: true });
  await writeFile(
    join(agentDir, "settings.json"),
    JSON.stringify({ packages: ["./fixture-package"] }),
  );
  await writeFile(
    join(pkg, "package.json"),
    JSON.stringify({ name: "fixture" }),
  );
  await writeFile(prompt, "prompt");
  await writeFile(theme, "{}");
  await writeFile(nestedIgnore, "small");

  await isolatedHome(root, async () => {
    const snapshot = scan(cwd, agentDir);
    const paths = snapshot.fingerprints.map((entry) => entry.path);
    assert.ok(paths.includes(prompt));
    assert.ok(paths.includes(theme));
    assert.ok(paths.includes(nestedIgnore));

    await writeFile(nestedIgnore, "x".repeat(1_025));
    assert.throws(
      () => scan(cwd, agentDir, { maxResourceFileBytes: 1_024 }),
      /BUZZ_PI_RESOURCE_LIMIT: prompt ignore manifest exceeds/,
    );
  });
});

test("package extension glob traversal is bounded without charging code bytes", async () => {
  const { root, cwd, agentDir } = await fixture("buzz-pi-package-extension-");
  const pkg = join(agentDir, "extension-package");
  const nested = join(pkg, "extensions", "one", "two");
  const ignore = join(nested, ".gitignore");
  await mkdir(nested, { recursive: true });
  await writeFile(
    join(agentDir, "settings.json"),
    JSON.stringify({ packages: ["./extension-package"] }),
  );
  await writeFile(
    join(pkg, "package.json"),
    JSON.stringify({
      name: "extension-package",
      pi: { extensions: ["extensions/**/*.js"] },
    }),
  );
  await writeFile(join(nested, "large.js"), "x".repeat(8_192));
  await writeFile(ignore, "small");

  await isolatedHome(root, async () => {
    const snapshot = scan(cwd, agentDir, { maxResourceFileBytes: 1_024 });
    assert.ok(snapshot.fingerprints.some((entry) => entry.path === ignore));
    assert.ok(
      !snapshot.fingerprints.some((entry) => entry.path.endsWith("large.js")),
      "extension source is trusted executable code, not non-code byte budget",
    );
    await writeFile(ignore, "x".repeat(1_025));
    assert.throws(
      () => scan(cwd, agentDir, { maxResourceFileBytes: 1_024 }),
      /BUZZ_PI_RESOURCE_LIMIT: extension ignore manifest exceeds/,
    );
  });
});

test("configured legacy global npm packages retain regular Pi resource compatibility", async () => {
  const { root, cwd, agentDir } = await fixture("buzz-pi-legacy-npm-");
  const globalRoot = join(root, "legacy-global", "node_modules");
  const pkg = join(globalRoot, "legacy-fixture");
  const nestedPrompt = join(pkg, "prompts", "nested", "legacy.md");
  const resolver = join(root, "npm-root.cjs");
  await mkdir(join(pkg, "prompts", "nested"), { recursive: true });
  await writeFile(
    resolver,
    `process.stdout.write(${JSON.stringify(globalRoot)} + "\\n");`,
  );
  await writeFile(
    join(agentDir, "settings.json"),
    JSON.stringify({
      npmCommand: [process.execPath, resolver],
      packages: ["npm:legacy-fixture@1.0.0  "],
    }),
  );
  await writeFile(
    join(pkg, "package.json"),
    JSON.stringify({ name: "legacy-fixture", version: "1.0.0" }),
  );
  await writeFile(nestedPrompt, "legacy prompt");

  await isolatedHome(root, async () => {
    const snapshot = scan(cwd, agentDir);
    assert.ok(
      snapshot.fingerprints.some((entry) => entry.path === nestedPrompt),
    );
  });
});

test("configured deep Git package roots bypass no resources through intermediate manifests", async () => {
  const { root, cwd, agentDir } = await fixture("buzz-pi-managed-git-");
  const pkg = join(
    agentDir,
    "git",
    "gitlab.example",
    "group",
    "subgroup",
    "team",
    "project",
  );
  const prompt = join(pkg, "prompts", "deep.md");
  await mkdir(join(pkg, "prompts"), { recursive: true });
  await writeFile(
    join(agentDir, "settings.json"),
    JSON.stringify({
      packages: ["git:https://gitlab.example/group/subgroup/team/project"],
    }),
  );
  await writeFile(
    join(agentDir, "git", "gitlab.example", "package.json"),
    JSON.stringify({ name: "stray-intermediate" }),
  );
  await writeFile(join(pkg, "package.json"), JSON.stringify({ name: "deep" }));
  await writeFile(prompt, "deep prompt");
  await isolatedHome(root, async () => {
    const snapshot = scan(cwd, agentDir);
    assert.ok(snapshot.fingerprints.some((entry) => entry.path === prompt));
  });
});

test("hosted Git shorthand refs scan the exact managed package root", async () => {
  const { root, cwd, agentDir } = await fixture("buzz-pi-hosted-git-ref-");
  const pkg = join(agentDir, "git", "github.com", "user", "repo");
  const prompt = join(pkg, "prompts", "oversized.md");
  await mkdir(join(pkg, "prompts"), { recursive: true });
  await writeFile(
    join(agentDir, "settings.json"),
    JSON.stringify({ packages: ["  git:github:user/repo#main  "] }),
  );
  await writeFile(
    join(agentDir, "git", "github.com", "package.json"),
    JSON.stringify({ name: "stray-intermediate" }),
  );
  await writeFile(join(pkg, "package.json"), JSON.stringify({ name: "repo" }));
  await writeFile(prompt, "x".repeat(257));

  await isolatedHome(root, async () => {
    assert.throws(
      () => scan(cwd, agentDir, { maxResourceFileBytes: 256 }),
      /BUZZ_PI_RESOURCE_LIMIT: prompt exceeds the per-file limit/,
    );
  });
});

test("bare, leading-space npm, nested, and malformed package sources fall back to scope-relative paths", async () => {
  const { root, cwd, agentDir } = await fixture("buzz-pi-local-source-parity-");
  const sources = [
    ["  my-package  ", join(agentDir, "my-package")],
    ["subdir/pkg", join(agentDir, "subdir", "pkg")],
    ["github:user/repo", join(agentDir, "github:user", "repo")],
    ["https:not-a-url/pkg", join(agentDir, "https:not-a-url", "pkg")],
    ["  npm:local-package  ", join(agentDir, "npm:local-package")],
  ];
  const prompts = [];
  for (const [source, pkg] of sources) {
    const prompt = join(pkg, "prompts", "local.md");
    await mkdir(join(pkg, "prompts"), { recursive: true });
    await writeFile(
      join(pkg, "package.json"),
      JSON.stringify({ name: source.trim() }),
    );
    await writeFile(prompt, `prompt:${source.trim()}`);
    prompts.push(prompt);
  }
  await writeFile(
    join(agentDir, "settings.json"),
    JSON.stringify({ packages: sources.map(([source]) => source) }),
  );

  await isolatedHome(root, async () => {
    const before = scan(cwd, agentDir);
    const paths = new Set(before.fingerprints.map((entry) => entry.path));
    for (const prompt of prompts) assert.ok(paths.has(prompt));

    await writeFile(prompts[0], "changed bare package prompt");
    const changed = scan(cwd, agentDir);
    assert.throws(
      () => assertPiResourceSnapshotsEqual(before, changed),
      /BUZZ_PI_RESOURCE_CHANGED/,
    );

    await writeFile(prompts.at(-1), "x".repeat(257));
    assert.throws(
      () => scan(cwd, agentDir, { maxResourceFileBytes: 256 }),
      /BUZZ_PI_RESOURCE_LIMIT: prompt exceeds the per-file limit/,
    );
  });
});

test("encoded file URLs and whitespace-padded configured resource paths resolve natively", async () => {
  const { root, cwd, agentDir } = await fixture("buzz-pi-configured-paths-");
  const pkg = join(root, "package with spaces");
  const packagePrompt = join(pkg, "prompts", "package.md");
  const skill = join(agentDir, "custom skills", "review", "SKILL.md");
  const prompt = join(agentDir, "custom prompts", "review.md");
  const theme = join(agentDir, "custom themes", "review.json");
  const extensionIgnore = join(agentDir, "custom extensions", ".ignore");
  await mkdir(join(pkg, "prompts"), { recursive: true });
  await mkdir(join(agentDir, "custom skills", "review"), { recursive: true });
  await mkdir(join(agentDir, "custom prompts"), { recursive: true });
  await mkdir(join(agentDir, "custom themes"), { recursive: true });
  await mkdir(join(agentDir, "custom extensions"), { recursive: true });
  await writeFile(
    join(pkg, "package.json"),
    JSON.stringify({ name: "spaces" }),
  );
  await writeFile(packagePrompt, "package prompt");
  await writeFile(skill, "---\ndescription: review\n---\n");
  await writeFile(prompt, "custom prompt");
  await writeFile(theme, "{}");
  await writeFile(extensionIgnore, "small");
  await writeFile(
    join(agentDir, "settings.json"),
    JSON.stringify({
      packages: [`  ${pathToFileURL(pkg).href}  `],
      skills: ["  ./custom skills  "],
      prompts: ["  ./custom prompts  "],
      themes: ["  ./custom themes  "],
      extensions: ["  ./custom extensions  "],
    }),
  );

  await isolatedHome(root, async () => {
    const snapshot = scan(cwd, agentDir);
    const paths = new Set(snapshot.fingerprints.map((entry) => entry.path));
    for (const expected of [
      packagePrompt,
      skill,
      prompt,
      theme,
      extensionIgnore,
    ]) {
      assert.ok(paths.has(expected));
    }
  });
});

test("resource aggregate bytes, file count, entry count, and depth fail closed", async () => {
  const aggregate = await fixture("buzz-pi-resource-aggregate-");
  await mkdir(join(aggregate.agentDir, "prompts"));
  await writeFile(
    join(aggregate.agentDir, "prompts", "one.md"),
    "x".repeat(40),
  );
  await writeFile(
    join(aggregate.agentDir, "prompts", "two.md"),
    "x".repeat(40),
  );
  await isolatedHome(aggregate.root, async () => {
    assert.throws(
      () =>
        scan(aggregate.cwd, aggregate.agentDir, {
          maxResourceFileBytes: 64,
          maxResourceTotalBytes: 70,
        }),
      /BUZZ_PI_RESOURCE_LIMIT: non-code resource bytes exceed 70/,
    );
    assert.throws(
      () =>
        scan(aggregate.cwd, aggregate.agentDir, {
          maxResourceFiles: 1,
        }),
      /BUZZ_PI_RESOURCE_LIMIT: non-code resource file count exceeds 1/,
    );
  });

  const entries = await fixture("buzz-pi-resource-entries-");
  await mkdir(join(entries.agentDir, "prompts"));
  for (let index = 0; index < 5; index += 1) {
    await writeFile(
      join(entries.agentDir, "prompts", `ignored-${index}.txt`),
      "",
    );
  }
  await isolatedHome(entries.root, async () => {
    assert.throws(
      () =>
        scan(entries.cwd, entries.agentDir, {
          maxResourceFiles: 1,
          maxResourceEntries: 4,
        }),
      /BUZZ_PI_RESOURCE_LIMIT: prompt discovery entries exceed 4/,
    );
  });

  const depth = await fixture("buzz-pi-resource-depth-");
  await mkdir(join(depth.agentDir, "skills", "one", "two"), {
    recursive: true,
  });
  await writeFile(
    join(depth.agentDir, "skills", "one", "two", "SKILL.md"),
    "---\ndescription: nested\n---\n",
  );
  await isolatedHome(depth.root, async () => {
    assert.throws(
      () =>
        scan(depth.cwd, depth.agentDir, {
          maxResourceDepth: 1,
        }),
      /BUZZ_PI_RESOURCE_LIMIT: skill traversal exceeds depth 1/,
    );
  });
});

test("resource fingerprints detect same-path growth and inode replacement", async () => {
  const { root, cwd, agentDir } = await fixture(
    "buzz-pi-resource-fingerprint-",
  );
  const prompt = join(agentDir, "prompts", "stable.md");
  await mkdir(join(agentDir, "prompts"));
  await writeFile(prompt, "before");
  await isolatedHome(root, async () => {
    const before = scan(cwd, agentDir);
    await writeFile(prompt, "before plus growth");
    const grown = scan(cwd, agentDir);
    assert.throws(
      () => assertPiResourceSnapshotsEqual(before, grown),
      /BUZZ_PI_RESOURCE_CHANGED/,
    );

    const replacementPath = join(agentDir, "prompts", "replacement.tmp");
    await writeFile(replacementPath, "before");
    await rename(replacementPath, prompt);
    const replacement = scan(cwd, agentDir);
    assert.throws(
      () => assertPiResourceSnapshotsEqual(before, replacement),
      /BUZZ_PI_RESOURCE_CHANGED/,
    );
  });
});
