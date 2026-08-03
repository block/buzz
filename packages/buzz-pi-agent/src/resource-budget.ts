import { createHash } from "node:crypto";
import { execFileSync } from "node:child_process";
import {
  closeSync,
  constants,
  existsSync,
  fstatSync,
  openSync,
  opendirSync,
  readSync,
  realpathSync,
  statSync,
} from "node:fs";
import type { Dirent } from "node:fs";
import { homedir } from "node:os";
import { basename, dirname, isAbsolute, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import type { AdapterConfig } from "./config.js";

type ResourceMode = "skills" | "prompts" | "themes";

type ClassifiedPackageSource =
  | { type: "npm"; name: string | undefined }
  | { type: "git"; host: string; path: string }
  | { type: "local"; path: string };

export interface ResourceBudgetSnapshot {
  files: number;
  entries: number;
  bytes: number;
  fingerprints: readonly ResourceFileFingerprint[];
}

export interface ResourceFileFingerprint {
  path: string;
  canonicalPath: string;
  device: string;
  inode: string;
  size: number;
  mtimeNs: string;
  ctimeNs: string;
  sha256: string;
}

interface ResourceBudgetOptions {
  cwd: string;
  agentDir: string;
  projectTrusted: boolean;
  config: AdapterConfig;
  systemPromptSource?: string;
}

/**
 * Preflight every normal non-code file tree Pi will synchronously discover.
 * Extensions remain trusted executable code; their arbitrary imports cannot be
 * contained by a file scanner and are covered by the documented worker boundary.
 */
export function assertPiResourceBudget(
  options: ResourceBudgetOptions,
): ResourceBudgetSnapshot {
  const budget = new ResourceBudget(options.config);
  const globalSettings = budget.readJson(
    join(options.agentDir, "settings.json"),
    "global settings",
  );
  budget.readJson(join(options.agentDir, "models.json"), "model manifest");
  budget.checkFile(join(options.agentDir, "auth.json"), "credential manifest");
  budget.readJson(
    join(options.agentDir, "trust.json"),
    "project trust manifest",
  );
  if (options.systemPromptSource && existsSync(options.systemPromptSource)) {
    budget.checkFile(options.systemPromptSource, "explicit system prompt");
  }

  let projectSettings: Record<string, unknown> | undefined;
  if (options.projectTrusted) {
    projectSettings = budget.readJson(
      join(options.cwd, ".pi", "settings.json"),
      "project settings",
    );
  }

  scanContextFiles(budget, options.cwd, options.agentDir);
  scanScopeResources(budget, options.agentDir, globalSettings);
  budget.scanDirectory(join(homedir(), ".agents", "skills"), "skills");
  scanExtensionManifests(budget, join(options.agentDir, "extensions"));
  scanManagedPackages(budget, options.agentDir, globalSettings, true);

  if (options.projectTrusted) {
    const projectDir = join(options.cwd, ".pi");
    scanScopeResources(budget, projectDir, projectSettings);
    scanExtensionManifests(budget, join(projectDir, "extensions"));
    scanManagedPackages(budget, projectDir, projectSettings, false);
    for (const skillsDir of ancestorAgentsSkillDirs(options.cwd)) {
      budget.scanDirectory(skillsDir, "skills");
    }
  }

  return budget.snapshot();
}

/**
 * Verify that discovery and every bounded file byte stayed identical across a
 * Pi SDK load. This catches same-path growth, in-place rewrites, replacement,
 * symlink swaps, additions, and removals in the preflight/load window.
 */
export function assertPiResourceSnapshotsEqual(
  before: ResourceBudgetSnapshot,
  after: ResourceBudgetSnapshot,
): void {
  if (
    before.files !== after.files ||
    before.entries !== after.entries ||
    before.bytes !== after.bytes ||
    before.fingerprints.length !== after.fingerprints.length
  ) {
    throw resourceChanged("non-code resource discovery changed during Pi load");
  }
  for (let index = 0; index < before.fingerprints.length; index += 1) {
    const left = before.fingerprints[index];
    const right = after.fingerprints[index];
    if (!left || !right || !sameFingerprint(left, right)) {
      throw resourceChanged("a non-code resource changed during Pi load");
    }
  }
}

class ResourceBudget {
  private readonly checkedFiles = new Map<
    string,
    { fingerprint: ResourceFileFingerprint; content: Buffer }
  >();
  private files = 0;
  private entries = 0;
  private bytes = 0;

  constructor(private readonly config: AdapterConfig) {}

  snapshot(): ResourceBudgetSnapshot {
    return {
      files: this.files,
      entries: this.entries,
      bytes: this.bytes,
      fingerprints: [...this.checkedFiles.values()]
        .map(({ fingerprint }) => ({ ...fingerprint }))
        .sort((left, right) => left.path.localeCompare(right.path)),
    };
  }

  checkFile(path: string, label: string): boolean {
    return this.checkedFile(path, label) !== undefined;
  }

  readJson(path: string, label: string): Record<string, unknown> | undefined {
    const checked = this.checkedFile(path, label);
    if (!checked) return undefined;
    try {
      const parsed: unknown = JSON.parse(checked.content.toString("utf8"));
      return isRecord(parsed) ? parsed : undefined;
    } catch {
      // Pi owns schema/parse diagnostics. The budget only guarantees that the
      // attempted parse is bounded and cannot exhaust the runtime host.
      return undefined;
    }
  }

  scanPath(path: string, mode: ResourceMode, recursive = false): void {
    if (!existsSync(path)) return;
    let stats: ReturnType<typeof statSync>;
    try {
      stats = statSync(path);
    } catch {
      return;
    }
    if (stats.isFile()) {
      this.checkFile(path, resourceLabel(mode));
    } else if (stats.isDirectory()) {
      this.scanDirectory(path, mode, recursive);
    }
  }

  scanDirectory(path: string, mode: ResourceMode, recursive = false): void {
    this.scanDirectoryInner(path, mode, 0, new Set(), recursive);
  }

  scanExtensionDiscovery(path: string, recursive: boolean): void {
    if (!existsSync(path)) return;
    let stats: ReturnType<typeof statSync>;
    try {
      stats = statSync(path);
    } catch {
      return;
    }
    if (!stats.isDirectory()) return;
    this.scanExtensionDirectoryInner(path, 0, new Set(), recursive);
  }

  listDirectory(path: string, label: string): string[] {
    return this.readDirectory(path, label).map((entry) => entry.name);
  }

  private scanDirectoryInner(
    path: string,
    mode: ResourceMode,
    depth: number,
    ancestors: Set<string>,
    recursive: boolean,
  ): void {
    if (depth > this.config.maxResourceDepth) {
      throw resourceLimit(
        `${resourceLabel(mode)} traversal exceeds depth ${this.config.maxResourceDepth}`,
      );
    }
    let canonical: string;
    let stats: ReturnType<typeof statSync>;
    try {
      canonical = realpathSync.native(path);
      stats = statSync(path);
    } catch {
      return;
    }
    if (!stats.isDirectory()) {
      if (stats.isFile()) this.checkFile(path, resourceLabel(mode));
      return;
    }
    if (ancestors.has(canonical)) {
      throw resourceLimit(
        `${resourceLabel(mode)} traversal contains a symlink cycle`,
      );
    }
    const nextAncestors = new Set(ancestors).add(canonical);
    const entries = this.readDirectory(path, resourceLabel(mode));
    checkIgnoreManifests(this, path, `${resourceLabel(mode)} ignore manifest`);

    if (mode === "skills") {
      const rootSkill = entries.find((entry) => entry.name === "SKILL.md");
      if (rootSkill) {
        this.checkFile(join(path, rootSkill.name), "skill");
        return;
      }
    }

    for (const entry of entries) {
      if (entry.name.startsWith(".")) continue;
      if (entry.name === "node_modules") continue;
      const child = join(path, entry.name);
      if (mode === "skills") {
        if (entry.name === "node_modules") continue;
        let childStats: ReturnType<typeof statSync>;
        try {
          childStats = statSync(child);
        } catch {
          continue;
        }
        if (childStats.isDirectory()) {
          this.scanDirectoryInner(
            child,
            mode,
            depth + 1,
            nextAncestors,
            recursive,
          );
        } else if (childStats.isFile() && entry.name.endsWith(".md")) {
          this.checkFile(child, "skill");
        }
        continue;
      }
      let childStats: ReturnType<typeof statSync>;
      try {
        childStats = statSync(child);
      } catch {
        continue;
      }
      if (recursive && childStats.isDirectory()) {
        this.scanDirectoryInner(child, mode, depth + 1, nextAncestors, true);
      } else if (
        childStats.isFile() &&
        mode === "prompts" &&
        entry.name.endsWith(".md")
      ) {
        this.checkFile(child, "prompt");
      } else if (
        childStats.isFile() &&
        mode === "themes" &&
        entry.name.endsWith(".json")
      ) {
        this.checkFile(child, "theme");
      }
    }
  }

  private checkedFile(
    path: string,
    label: string,
  ): { fingerprint: ResourceFileFingerprint; content: Buffer } | undefined {
    const lexicalPath = resolve(path);
    const existing = this.checkedFiles.get(lexicalPath);
    if (existing) return existing;
    const checked = captureResourceFile(
      lexicalPath,
      label,
      this.config.maxResourceFileBytes,
    );
    if (!checked) return undefined;
    const size = checked.fingerprint.size;
    if (size > this.config.maxResourceFileBytes) {
      throw resourceLimit(
        `${boundedLabel(label)} exceeds the per-file limit (${size} > ${this.config.maxResourceFileBytes} bytes)`,
      );
    }
    if (this.files + 1 > this.config.maxResourceFiles) {
      throw resourceLimit(
        `non-code resource file count exceeds ${this.config.maxResourceFiles}`,
      );
    }
    if (this.bytes + size > this.config.maxResourceTotalBytes) {
      throw resourceLimit(
        `non-code resource bytes exceed ${this.config.maxResourceTotalBytes}`,
      );
    }
    this.checkedFiles.set(lexicalPath, checked);
    this.files += 1;
    this.bytes += size;
    return checked;
  }

  private scanExtensionDirectoryInner(
    path: string,
    depth: number,
    ancestors: Set<string>,
    recursive: boolean,
  ): void {
    if (depth > this.config.maxResourceDepth) {
      throw resourceLimit(
        `extension discovery exceeds depth ${this.config.maxResourceDepth}`,
      );
    }
    let canonical: string;
    try {
      canonical = realpathSync.native(path);
    } catch {
      return;
    }
    if (ancestors.has(canonical)) {
      throw resourceLimit("extension discovery contains a symlink cycle");
    }
    const nextAncestors = new Set(ancestors).add(canonical);
    checkIgnoreManifests(this, path, "extension ignore manifest");
    this.readJson(join(path, "package.json"), "extension manifest");
    const entries = this.readDirectory(path, "extension discovery");
    for (const entry of entries) {
      if (entry.name.startsWith(".") || entry.name === "node_modules") continue;
      const child = join(path, entry.name);
      let childStats: ReturnType<typeof statSync>;
      try {
        childStats = statSync(child);
      } catch {
        continue;
      }
      if (!childStats.isDirectory()) continue;
      if (recursive) {
        this.scanExtensionDirectoryInner(child, depth + 1, nextAncestors, true);
      } else {
        this.readJson(join(child, "package.json"), "extension manifest");
      }
    }
  }

  private readDirectory(path: string, label: string): Dirent<string>[] {
    if (!existsSync(path)) return [];
    let directory: ReturnType<typeof opendirSync>;
    try {
      directory = opendirSync(path, { encoding: "utf8" });
    } catch {
      return [];
    }
    const entries: Dirent<string>[] = [];
    try {
      while (true) {
        const entry = directory.readSync();
        if (entry === null) break;
        this.addEntries(1, label);
        entries.push(entry);
      }
      return entries;
    } finally {
      directory.closeSync();
    }
  }

  private addEntries(count: number, label: string): void {
    if (this.entries + count > this.config.maxResourceEntries) {
      throw resourceLimit(
        `${boundedLabel(label)} discovery entries exceed ${this.config.maxResourceEntries}`,
      );
    }
    this.entries += count;
  }
}

function scanScopeResources(
  budget: ResourceBudget,
  baseDir: string,
  settings: Record<string, unknown> | undefined,
): void {
  budget.scanDirectory(join(baseDir, "skills"), "skills");
  budget.scanDirectory(join(baseDir, "prompts"), "prompts");
  budget.scanDirectory(join(baseDir, "themes"), "themes");
  budget.checkFile(join(baseDir, "SYSTEM.md"), "system prompt");
  budget.checkFile(join(baseDir, "APPEND_SYSTEM.md"), "appended system prompt");

  for (const mode of ["skills", "prompts", "themes"] as const) {
    for (const configuredPath of stringEntries(settings?.[mode])) {
      const base = fixedGlobPrefix(configuredPath);
      if (!base) continue;
      budget.scanPath(resolveConfiguredPath(base, baseDir), mode, true);
    }
  }
  for (const configuredPath of stringEntries(settings?.extensions)) {
    const base = fixedGlobPrefix(configuredPath);
    if (!base) continue;
    budget.scanExtensionDiscovery(
      resolveConfiguredPath(base, baseDir),
      containsGlob(configuredPath),
    );
  }
  for (const pkg of arrayEntries(settings?.packages)) {
    const source = packageSource(pkg);
    if (source === undefined) continue;
    const classified = classifyPackageSource(source);
    if (classified.type !== "local") continue;
    scanPackageRoot(budget, resolveConfiguredPath(classified.path, baseDir));
  }
}

function scanContextFiles(
  budget: ResourceBudget,
  cwd: string,
  agentDir: string,
): void {
  checkFirstContextFile(budget, agentDir);
  let current = cwd;
  while (true) {
    checkFirstContextFile(budget, current);
    const parent = dirname(current);
    if (parent === current) break;
    current = parent;
  }
}

function checkFirstContextFile(budget: ResourceBudget, dir: string): void {
  for (const name of ["AGENTS.md", "AGENTS.MD", "CLAUDE.md", "CLAUDE.MD"]) {
    const path = join(dir, name);
    if (!existsSync(path)) continue;
    if (budget.checkFile(path, "context file")) return;
  }
}

function scanExtensionManifests(budget: ResourceBudget, root: string): void {
  checkIgnoreManifests(budget, root, "extension ignore manifest");
  budget.readJson(join(root, "package.json"), "extension manifest");
  for (const name of budget.listDirectory(root, "extension discovery")) {
    const child = join(root, name);
    let stats: ReturnType<typeof statSync>;
    try {
      stats = statSync(child);
    } catch {
      continue;
    }
    if (stats.isDirectory()) {
      budget.readJson(join(child, "package.json"), "extension manifest");
    }
  }
}

function checkIgnoreManifests(
  budget: ResourceBudget,
  dir: string,
  label: string,
): void {
  for (const name of [".gitignore", ".ignore", ".fdignore"]) {
    budget.checkFile(join(dir, name), label);
  }
}

function scanManagedPackages(
  budget: ResourceBudget,
  baseDir: string,
  settings: Record<string, unknown> | undefined,
  allowLegacyGlobal: boolean,
): void {
  const npmRoot = join(baseDir, "npm", "node_modules");
  for (const name of configuredNpmPackageNames(settings)) {
    const managedPath = join(npmRoot, name);
    if (existsSync(managedPath)) {
      scanPackageRoot(budget, managedPath);
      continue;
    }
    if (!allowLegacyGlobal) continue;
    const legacyPath = resolveLegacyGlobalNpmPackage(name, settings);
    if (legacyPath) scanPackageRoot(budget, legacyPath);
  }
  for (const root of configuredGitPackageRoots(settings, baseDir)) {
    scanPackageRoot(budget, root);
  }
}

function configuredNpmPackageNames(
  settings: Record<string, unknown> | undefined,
): string[] {
  const names = new Set<string>();
  for (const pkg of arrayEntries(settings?.packages)) {
    const source = packageSource(pkg);
    if (source === undefined) continue;
    const classified = classifyPackageSource(source);
    if (classified.type === "npm" && classified.name)
      names.add(classified.name);
  }
  return [...names];
}

function configuredGitPackageRoots(
  settings: Record<string, unknown> | undefined,
  baseDir: string,
): string[] {
  const roots = new Set<string>();
  for (const pkg of arrayEntries(settings?.packages)) {
    const source = packageSource(pkg);
    if (source === undefined) continue;
    const classified = classifyPackageSource(source);
    if (classified.type === "git") {
      roots.add(join(baseDir, "git", classified.host, classified.path));
    }
  }
  return [...roots];
}

/**
 * Match Pi 0.83's source ordering: npm first, then known-local syntax, then
 * supported Git syntax, with every unrecognized value falling back to a path
 * relative to the package's settings scope. The trim is also the one Pi uses
 * when it resolves local package paths.
 */
function classifyPackageSource(source: string): ClassifiedPackageSource {
  const trimmed = source.trim();
  // Pi checks npm against the original string before any path normalization.
  // A leading-space ` npm:foo` therefore falls through to a scope-relative
  // local path, while `npm:foo ` remains a managed npm package.
  if (source.startsWith("npm:")) {
    return { type: "npm", name: parseNpmPackageName(trimmed) };
  }
  if (isPiLocalPackagePath(trimmed)) {
    return { type: "local", path: trimmed };
  }
  const git = parseManagedGitLocation(trimmed);
  return git ? { type: "git", ...git } : { type: "local", path: trimmed };
}

function parseNpmPackageName(source: string): string | undefined {
  const spec = source.slice("npm:".length).trim();
  let name = spec;
  if (spec.startsWith("@")) {
    const versionSeparator = spec.indexOf("@", spec.indexOf("/") + 1);
    if (versionSeparator >= 0) name = spec.slice(0, versionSeparator);
  } else {
    const versionSeparator = spec.indexOf("@");
    if (versionSeparator >= 0) name = spec.slice(0, versionSeparator);
  }
  return name &&
    !name.includes("..") &&
    !name.includes("\\") &&
    !isAbsolute(name)
    ? name
    : undefined;
}

function parseManagedGitLocation(
  source: string,
): { host: string; path: string } | undefined {
  const trimmed = source.trim();
  const explicitGit = trimmed.startsWith("git:");
  const value = explicitGit ? trimmed.slice(4).trim() : trimmed;
  if (!explicitGit && !/^(https?|ssh|git):\/\//iu.test(value)) return undefined;

  const hosted = parseHostedGitLocation(value);
  if (hosted) return validateManagedGitLocation(hosted.host, hosted.path);

  let host: string;
  let path: string;
  const scp = value.match(/^git@([^:]+):(.+)$/u);
  if (scp) {
    host = scp[1] ?? "";
    path = scp[2] ?? "";
  } else if (/^(https?|ssh|git):\/\//iu.test(value)) {
    try {
      const url = new URL(value);
      host = url.hostname;
      path = url.pathname.replace(/^\/+/, "");
    } catch {
      return undefined;
    }
  } else {
    const slash = value.indexOf("/");
    if (slash < 0) return undefined;
    host = value.slice(0, slash);
    path = value.slice(slash + 1);
    if (!host.includes(".") && host !== "localhost") return undefined;
  }
  const refSeparator = path.indexOf("@");
  if (refSeparator >= 0) path = path.slice(0, refSeparator);
  return validateManagedGitLocation(host, path);
}

function parseHostedGitLocation(
  value: string,
): { host: string; path: string } | undefined {
  const shorthand = value.match(/^(github|gitlab|bitbucket|gist):(.+)$/iu);
  if (shorthand) {
    const service = shorthand[1]?.toLowerCase();
    const host =
      service === "github"
        ? "github.com"
        : service === "gitlab"
          ? "gitlab.com"
          : service === "bitbucket"
            ? "bitbucket.org"
            : "gist.github.com";
    return { host, path: stripHostedGitReference(shorthand[2] ?? "") };
  }

  const scp = value.match(/^git@([^:]+):(.+)$/u);
  if (scp && isHostedGitDomain(scp[1] ?? "")) {
    return {
      host: scp[1] ?? "",
      path: stripHostedGitReference(scp[2] ?? ""),
    };
  }

  if (/^[a-z][a-z+.-]*:\/\//iu.test(value)) {
    try {
      const url = new URL(value);
      if (isHostedGitDomain(url.hostname)) {
        return {
          host: url.hostname,
          path: stripHostedGitReference(url.pathname.replace(/^\/+/, "")),
        };
      }
    } catch {
      return undefined;
    }
  }

  const slash = value.indexOf("/");
  if (slash >= 0) {
    const host = value.slice(0, slash);
    if (isHostedGitDomain(host)) {
      return {
        host,
        path: stripHostedGitReference(value.slice(slash + 1)),
      };
    }
  }

  // hosted-git-info treats the historical `git:user/repo` shortcut as a
  // GitHub repository. More deeply nested bare values are generic Git paths.
  const shortcut = stripHostedGitReference(value).split("/");
  if (shortcut.length === 2 && shortcut.every(Boolean)) {
    return { host: "github.com", path: shortcut.join("/") };
  }
  return undefined;
}

function stripHostedGitReference(path: string): string {
  const hashSeparator = path.indexOf("#");
  const withoutHash = hashSeparator >= 0 ? path.slice(0, hashSeparator) : path;
  const slash = withoutHash.indexOf("/");
  const atSeparator = withoutHash.indexOf("@", slash + 1);
  return atSeparator >= 0 ? withoutHash.slice(0, atSeparator) : withoutHash;
}

function isHostedGitDomain(host: string): boolean {
  return (
    host === "github.com" ||
    host === "gitlab.com" ||
    host === "bitbucket.org" ||
    host === "gist.github.com"
  );
}

function validateManagedGitLocation(
  host: string,
  rawPath: string,
): { host: string; path: string } | undefined {
  const path = rawPath.replace(/\.git$/u, "").replace(/^\/+/, "");
  if (
    !host ||
    path.split("/").length < 2 ||
    hasUnsafeGitInstallPart(host, false) ||
    hasUnsafeGitInstallPart(path, true)
  ) {
    return undefined;
  }
  return { host, path };
}

function hasUnsafeGitInstallPart(value: string, allowSlash: boolean): boolean {
  let decoded: string;
  try {
    decoded = decodeURIComponent(value);
  } catch {
    return true;
  }
  for (const candidate of [value, decoded]) {
    if (
      candidate.includes("\0") ||
      candidate.includes("\\") ||
      candidate.startsWith("/") ||
      (!allowSlash && candidate.includes("/")) ||
      candidate.split("/").includes("..")
    ) {
      return true;
    }
  }
  return false;
}

function resolveLegacyGlobalNpmPackage(
  packageName: string,
  settings: Record<string, unknown> | undefined,
): string | undefined {
  const configured = stringEntries(settings?.npmCommand);
  const command = configured[0] ?? "npm";
  const prefixArgs = configured.slice(1);
  const commandParts = [command, ...prefixArgs];
  const separator = commandParts.lastIndexOf("--");
  const packageManagerCommand =
    separator >= 0 ? commandParts[separator + 1] : command;
  const packageManager = packageManagerCommand
    ? basename(packageManagerCommand).replace(/\.(cmd|exe)$/iu, "")
    : "";
  try {
    if (packageManager === "pnpm") {
      const output = runPackageManager(command, prefixArgs, [
        "list",
        "-g",
        "--depth",
        "0",
        "--json",
      ]);
      const parsed: unknown = JSON.parse(output);
      if (!Array.isArray(parsed)) return undefined;
      for (const entry of parsed) {
        if (!isRecord(entry) || !isRecord(entry.dependencies)) continue;
        const dependency = entry.dependencies[packageName];
        if (isRecord(dependency) && typeof dependency.path === "string") {
          return resolve(dependency.path);
        }
      }
      return undefined;
    }
    if (packageManager === "bun") {
      const binDir = runPackageManager(command, prefixArgs, [
        "pm",
        "bin",
        "-g",
      ]).trim();
      return binDir
        ? join(
            dirname(resolve(binDir)),
            "install",
            "global",
            "node_modules",
            packageName,
          )
        : undefined;
    }
    const root = runPackageManager(command, prefixArgs, ["root", "-g"]).trim();
    return root ? join(resolve(root), packageName) : undefined;
  } catch {
    // Pi also treats an unavailable legacy root as missing and may install the
    // configured package into its managed store. The post-load fingerprint
    // comparison then requires a fresh session for that new generation.
    return undefined;
  }
}

function runPackageManager(
  command: string,
  prefixArgs: string[],
  args: string[],
): string {
  return execFileSync(command, [...prefixArgs, ...args], {
    encoding: "utf8",
    timeout: 10_000,
    maxBuffer: 1 * 1_024 * 1_024,
    stdio: ["ignore", "pipe", "ignore"],
  });
}

function scanPackageRoot(budget: ResourceBudget, root: string): void {
  const manifest = budget.readJson(
    join(root, "package.json"),
    "package manifest",
  );
  checkIgnoreManifests(budget, root, "package ignore manifest");
  const pi = isRecord(manifest?.pi) ? manifest.pi : undefined;
  const configuredExtensions = stringEntries(pi?.extensions);
  if (configuredExtensions.length === 0) {
    budget.scanExtensionDiscovery(join(root, "extensions"), false);
  } else {
    for (const entry of configuredExtensions) {
      const prefix = fixedGlobPrefix(entry);
      if (prefix) {
        budget.scanExtensionDiscovery(
          resolve(prefix, root),
          containsGlob(entry),
        );
      }
    }
  }
  for (const mode of ["skills", "prompts", "themes"] as const) {
    const configured = stringEntries(pi?.[mode]);
    if (configured.length === 0) {
      budget.scanDirectory(join(root, mode), mode, true);
      continue;
    }
    for (const entry of configured) {
      const prefix = fixedGlobPrefix(entry);
      if (prefix) budget.scanPath(resolve(prefix, root), mode, true);
    }
  }
}

function ancestorAgentsSkillDirs(cwd: string): string[] {
  const result: string[] = [];
  let current = cwd;
  while (true) {
    const path = join(current, ".agents", "skills");
    if (existsSync(path)) result.push(path);
    const parent = dirname(current);
    if (parent === current) return result;
    current = parent;
  }
}

function fixedGlobPrefix(path: string): string | undefined {
  if (path.startsWith("!") || path.startsWith("+") || path.startsWith("-")) {
    return undefined;
  }
  const wildcard = path.search(/[?*[{]/);
  if (wildcard < 0) return path;
  const prefix = path.slice(0, wildcard);
  const slash = Math.max(prefix.lastIndexOf("/"), prefix.lastIndexOf("\\"));
  return slash < 0 ? "." : prefix.slice(0, slash) || ".";
}

function containsGlob(path: string): boolean {
  return /[?*[{]/u.test(path);
}

function resolveConfiguredPath(path: string, baseDir: string): string {
  const trimmed = path.trim();
  if (trimmed === "~") return homedir();
  if (trimmed.startsWith("~/")) {
    return join(homedir(), trimmed.slice(2));
  }
  const normalized = trimmed.startsWith("file://")
    ? fileURLToPath(trimmed)
    : trimmed;
  return isAbsolute(normalized)
    ? resolve(normalized)
    : resolve(baseDir, normalized);
}

function isPiLocalPackagePath(source: string): boolean {
  return !(
    source.startsWith("npm:") ||
    source.startsWith("git:") ||
    source.startsWith("github:") ||
    source.startsWith("http:") ||
    source.startsWith("https:") ||
    source.startsWith("ssh:")
  );
}

function packageSource(value: unknown): string | undefined {
  if (typeof value === "string") return value;
  return isRecord(value) && typeof value.source === "string"
    ? value.source
    : undefined;
}

function arrayEntries(value: unknown): unknown[] {
  return Array.isArray(value) ? value : [];
}

function stringEntries(value: unknown): string[] {
  return arrayEntries(value).filter(
    (entry): entry is string => typeof entry === "string",
  );
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

function resourceLabel(mode: ResourceMode): string {
  if (mode === "skills") return "skill";
  if (mode === "prompts") return "prompt";
  return "theme";
}

function boundedLabel(label: string): string {
  return label.replaceAll(/[^a-zA-Z0-9 _-]/g, "").slice(0, 64) || "resource";
}

function captureResourceFile(
  path: string,
  label: string,
  maxBytes: number,
): { fingerprint: ResourceFileFingerprint; content: Buffer } | undefined {
  let canonicalPath: string;
  try {
    canonicalPath = realpathSync.native(path);
  } catch {
    return undefined;
  }

  let descriptor: number | undefined;
  try {
    descriptor = openSync(
      canonicalPath,
      constants.O_RDONLY | (constants.O_NOFOLLOW ?? 0),
    );
    const before = fstatSync(descriptor, { bigint: true });
    if (!before.isFile()) return undefined;
    if (before.size > BigInt(maxBytes)) {
      throw resourceLimit(
        `${boundedLabel(label)} exceeds the per-file limit (${before.size.toString()} > ${maxBytes} bytes)`,
      );
    }
    const expectedBytes = Number(before.size);
    const content = Buffer.allocUnsafe(expectedBytes + 1);
    let bytesRead = 0;
    while (bytesRead < content.length) {
      const count = readSync(
        descriptor,
        content,
        bytesRead,
        content.length - bytesRead,
        null,
      );
      if (count === 0) break;
      bytesRead += count;
    }
    const after = fstatSync(descriptor, { bigint: true });
    const verifiedCanonicalPath = realpathSync.native(path);
    const pathStats = statSync(verifiedCanonicalPath, { bigint: true });
    if (
      bytesRead !== expectedBytes ||
      verifiedCanonicalPath !== canonicalPath ||
      before.dev !== after.dev ||
      before.ino !== after.ino ||
      before.dev !== pathStats.dev ||
      before.ino !== pathStats.ino ||
      before.size !== after.size ||
      before.size !== pathStats.size ||
      before.mtimeNs !== after.mtimeNs ||
      before.mtimeNs !== pathStats.mtimeNs ||
      before.ctimeNs !== after.ctimeNs ||
      before.ctimeNs !== pathStats.ctimeNs
    ) {
      throw resourceChanged("a non-code resource changed while it was read");
    }
    const boundedContent = content.subarray(0, bytesRead);
    return {
      fingerprint: {
        path,
        canonicalPath,
        device: before.dev.toString(),
        inode: before.ino.toString(),
        size: expectedBytes,
        mtimeNs: before.mtimeNs.toString(),
        ctimeNs: before.ctimeNs.toString(),
        sha256: createHash("sha256").update(boundedContent).digest("hex"),
      },
      content: boundedContent,
    };
  } catch (error) {
    if (
      error instanceof Error &&
      (error.message.startsWith("BUZZ_PI_RESOURCE_LIMIT:") ||
        error.message.startsWith("BUZZ_PI_RESOURCE_CHANGED:"))
    ) {
      throw error;
    }
    return undefined;
  } finally {
    if (descriptor !== undefined) closeSync(descriptor);
  }
}

function sameFingerprint(
  left: ResourceFileFingerprint,
  right: ResourceFileFingerprint,
): boolean {
  return (
    left.path === right.path &&
    left.canonicalPath === right.canonicalPath &&
    left.device === right.device &&
    left.inode === right.inode &&
    left.size === right.size &&
    left.mtimeNs === right.mtimeNs &&
    left.ctimeNs === right.ctimeNs &&
    left.sha256 === right.sha256
  );
}

function resourceLimit(message: string): Error {
  return new Error(`BUZZ_PI_RESOURCE_LIMIT: ${message.slice(0, 448)}`);
}

function resourceChanged(message: string): Error {
  return new Error(`BUZZ_PI_RESOURCE_CHANGED: ${message.slice(0, 448)}`);
}
