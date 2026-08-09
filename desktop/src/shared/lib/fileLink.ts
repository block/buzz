/**
 * `buzz://file` deep links for artifacts inside the Buzz nest, mirroring
 * `entityLink.ts` for `buzz://pr|issue|repo`.
 *
 * Format:
 *   buzz://file?path=<root-relative-path>[&root=nest|repos][&reveal=1]
 *
 * `path` is always relative to a *named* root — never absolute, and never
 * containing a `..` segment. `root` selects which one:
 *
 * - `nest` (the default) — the nest root, `~/.buzz` (`~/.buzz-dev` on dev
 *   builds).
 * - `repos` — the active workspace's repos directory, which the user may point
 *   outside the nest. It needs its own alias precisely because the nest's
 *   `REPOS` entry can be a symlink: a `nest`-rooted path that traverses it
 *   canonicalizes outside the nest and is rejected, by design.
 *
 * Only these two roots exist, and both are directories the app already manages.
 * The scheme deliberately cannot address an arbitrary absolute path; widening
 * it would turn any chat message into a filesystem opener.
 *
 * `reveal=1` selects "show in the file manager" over "open in the default
 * application".
 *
 * Agents write these links so a human can click an artifact in chat and land on
 * the live file rather than a copy pinned at upload time. The complementary
 * mechanism is `buzz messages send --file`, which uploads to Blossom and
 * renders a downloadable `FileCard`; the two are meant to be used together, so
 * a message stays useful on a device that does not have the nest.
 *
 * The validation here is a *convenience* filter, not the security boundary. The
 * real containment check lives in the `open_workspace_file` Tauri command,
 * which canonicalizes the joined path and requires the canonicalized root as a
 * prefix. Anything that can reach the IPC layer bypasses this module
 * entirely, so this file must never be the only thing standing between a link
 * and the filesystem.
 */

const FILE_LINK_SCHEME = "buzz:";

/** The named roots a file link may address. Must match `FileLinkRoot` in Rust. */
export const FILE_LINK_ROOTS = ["nest", "repos"] as const;

export type FileLinkRoot = (typeof FILE_LINK_ROOTS)[number];

export type ParsedFileLink = {
  path: string;
  root: FileLinkRoot;
  reveal: boolean;
};

export type FileLinkParseResult =
  | { ok: true; value: ParsedFileLink }
  | { ok: false; reason: string };

/** Upper bound on `path`, well clear of any real artifact path. */
const MAX_PATH_LENGTH = 1024;

function isFileLinkRoot(value: string): value is FileLinkRoot {
  return (FILE_LINK_ROOTS as readonly string[]).includes(value);
}

/**
 * Reject paths that are absolute, escape upward, or carry characters that have
 * no business in a root-relative artifact path.
 *
 * Backslashes are rejected rather than normalized: on Windows `a\..\b` would be
 * an escaping path that a POSIX-style `..` segment check does not catch, and
 * Buzz has no artifact paths that legitimately contain one.
 */
function isValidRootRelativePath(path: string): boolean {
  if (path.length === 0 || path.length > MAX_PATH_LENGTH) return false;
  if (path.includes("\0") || path.includes("\\")) return false;
  // Absolute POSIX path, or a Windows drive/UNC path.
  if (path.startsWith("/") || /^[a-zA-Z]:/.test(path)) return false;
  const segments = path.split("/");
  return segments.every((segment) => segment !== "" && segment !== "..");
}

/**
 * Build a `buzz://file` link for a root-relative artifact path.
 *
 * `root` is omitted from the emitted URL when it is the `nest` default, so the
 * common case stays short and the golden format has exactly one spelling.
 *
 * @throws if `path` is not a valid root-relative path.
 */
export function buildFileLink(input: {
  path: string;
  root?: FileLinkRoot;
  reveal?: boolean;
}): string {
  if (!isValidRootRelativePath(input.path)) {
    throw new Error(
      "fileLink: path must be a relative path inside a known root",
    );
  }
  const params = [`path=${encodeURIComponent(input.path)}`];
  if (input.root && input.root !== "nest") params.push(`root=${input.root}`);
  if (input.reveal) params.push("reveal=1");
  return `buzz://file?${params.join("&")}`;
}

/**
 * Cheap pre-check used by the markdown renderer and the URL transform before
 * parsing, matching `isEntityLink`'s role for git entities.
 */
export function isFileLink(href: string | undefined | null): boolean {
  if (!href) return false;
  return href.startsWith("buzz://file?");
}

/**
 * Parse a `buzz://file?…` URL. Returns a discriminated result so callers can
 * fall back to plain-link rendering without throwing.
 *
 * Strict canonical form, matching `parseEntityLink`, so that old clients
 * decline rather than silently misinterpret a future extension:
 * - Empty or root path only (no `/extra/segments`)
 * - No fragment
 * - `path` exactly once; `root` and `reveal` at most once each
 * - `root` must name a known root; `reveal` must be exactly `1`
 * - Unknown query parameters rejected
 */
export function parseFileLink(url: string): FileLinkParseResult {
  let parsed: URL;
  try {
    parsed = new URL(url);
  } catch {
    return { ok: false, reason: "invalid-url" };
  }

  if (parsed.protocol !== FILE_LINK_SCHEME) {
    return { ok: false, reason: "wrong-scheme" };
  }
  if (parsed.hostname !== "file") {
    return { ok: false, reason: "wrong-host" };
  }
  if (parsed.pathname !== "" && parsed.pathname !== "/") {
    return { ok: false, reason: "unexpected-path" };
  }
  if (parsed.hash) {
    return { ok: false, reason: "unexpected-fragment" };
  }

  const KNOWN_PARAMS = new Set(["path", "root", "reveal"]);
  for (const key of parsed.searchParams.keys()) {
    if (!KNOWN_PARAMS.has(key)) {
      return { ok: false, reason: "unknown-param" };
    }
    if (parsed.searchParams.getAll(key).length !== 1) {
      return { ok: false, reason: "duplicate-param" };
    }
  }

  const path = parsed.searchParams.get("path");
  if (path === null) return { ok: false, reason: "missing-path" };
  if (!isValidRootRelativePath(path)) return { ok: false, reason: "bad-path" };

  const rawRoot = parsed.searchParams.get("root");
  if (rawRoot !== null && !isFileLinkRoot(rawRoot)) {
    return { ok: false, reason: "unknown-root" };
  }
  const root: FileLinkRoot = rawRoot ?? "nest";

  const reveal = parsed.searchParams.get("reveal");
  if (reveal !== null && reveal !== "1") {
    return { ok: false, reason: "bad-reveal" };
  }

  return { ok: true, value: { path, root, reveal: reveal === "1" } };
}

/** Display label for a file link — the basename of its path. */
export function fileLinkBasename(link: ParsedFileLink): string {
  const segments = link.path.split("/");
  return segments[segments.length - 1] ?? link.path;
}
