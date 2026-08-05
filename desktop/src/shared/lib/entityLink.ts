/**
 * `buzz://` deep links for Buzz-hosted git entities, mirroring
 * `features/messages/lib/messageLink.ts` for `buzz://message`.
 *
 * Formats:
 *   buzz://repo?owner=<owner-pubkey>&d=<repo-dtag>
 *   buzz://pr?id=<event-id>&owner=<owner-pubkey>&d=<repo-dtag>
 *   buzz://issue?id=<event-id>&owner=<owner-pubkey>&d=<repo-dtag>
 *
 * `owner` + `d` identify the NIP-34 repository coordinate
 * (`30617:<owner>:<d>`); `id` is the kind 1618 / 1621 event id. The CLI
 * builder in `crates/buzz-cli/src/links.rs` emits the same format — the two
 * must stay compatible (see the golden-format tests on both sides).
 */

import {
  normalizeRepositoryPath,
  parseRepositoryRef,
  type RepositoryRefKind,
} from "./repositoryTarget.ts";

const ENTITY_LINK_SCHEME = "buzz:";

export type RepositoryEntityTarget = {
  ref: string;
  refKind: RepositoryRefKind;
  path: string;
};

export type ParsedEntityLink =
  | { type: "pr"; id: string; owner: string; dtag: string }
  | { type: "issue"; id: string; owner: string; dtag: string }
  | {
      type: "repo";
      owner: string;
      dtag: string;
      target?: RepositoryEntityTarget;
    };

export type EntityLinkParseResult =
  | { ok: true; value: ParsedEntityLink }
  | { ok: false; reason: string };

const HEX64_RE = /^[a-fA-F0-9]{64}$/;
const DTAG_RE = /^[a-zA-Z0-9._-]{1,64}$/;

function isValidDtag(dtag: string): boolean {
  return DTAG_RE.test(dtag) && !dtag.startsWith(".") && !dtag.includes("..");
}

function checkCoordinate(owner: string, dtag: string): void {
  if (!HEX64_RE.test(owner)) {
    throw new Error("entityLink: owner must be a 64-char hex pubkey");
  }
  if (!isValidDtag(dtag)) {
    throw new Error("entityLink: invalid repository d-tag");
  }
}

function checkEventId(id: string): void {
  if (!HEX64_RE.test(id)) {
    throw new Error("entityLink: id must be a 64-char hex event id");
  }
}

/** Build a `buzz://repo` link for a repository announcement (kind 30617). */
export function buildRepoLink(input: { owner: string; dtag: string }): string {
  checkCoordinate(input.owner, input.dtag);
  return `buzz://repo?owner=${input.owner.toLowerCase()}&d=${input.dtag}`;
}

/** Build a canonical repository link with an atomic ref/path target. */
export function buildRepoTargetLink(input: {
  owner: string;
  dtag: string;
  ref: string;
  path: string;
}): string {
  checkCoordinate(input.owner, input.dtag);
  const parsedRef = parseRepositoryRef(input.ref);
  const path = normalizeRepositoryPath(input.path);
  if (!parsedRef) throw new Error("entityLink: invalid repository ref");
  if (!path) throw new Error("entityLink: invalid repository path");
  const params = new URLSearchParams({
    owner: input.owner.toLowerCase(),
    d: input.dtag,
    ref: parsedRef.value,
    path,
  });
  return `buzz://repo?${params.toString()}`;
}

/** Build a `buzz://pr` link for a pull request event (kind 1618). */
export function buildPullRequestLink(input: {
  id: string;
  owner: string;
  dtag: string;
}): string {
  checkEventId(input.id);
  checkCoordinate(input.owner, input.dtag);
  return `buzz://pr?id=${input.id.toLowerCase()}&owner=${input.owner.toLowerCase()}&d=${input.dtag}`;
}

/** Build a `buzz://issue` link for an issue event (kind 1621). */
export function buildIssueLink(input: {
  id: string;
  owner: string;
  dtag: string;
}): string {
  checkEventId(input.id);
  checkCoordinate(input.owner, input.dtag);
  return `buzz://issue?id=${input.id.toLowerCase()}&owner=${input.owner.toLowerCase()}&d=${input.dtag}`;
}

/**
 * Cheap pre-check used by the markdown renderer and preview extraction
 * before parsing. `buzz://message` is intentionally excluded — it has its
 * own pill rendering path.
 */
export function isEntityLink(href: string | undefined | null): boolean {
  if (!href) return false;
  return (
    href.startsWith("buzz://pr?") ||
    href.startsWith("buzz://issue?") ||
    href.startsWith("buzz://repo?")
  );
}

/**
 * Parse a `buzz://pr|issue|repo?…` URL. Returns a discriminated result so
 * callers can fall back to plain-link rendering without throwing. All
 * identifiers are validated; hex values are lowercase-normalized.
 *
 * Strict canonical form to preserve forward-compatibility:
 * - Empty or root path only (no `/extra/segments`)
 * - No fragment
 * - Each required parameter must appear exactly once
 * - Unknown query parameters are rejected (callers that need to add
 *   parameters must version the format or add them to the known-params set)
 *
 * This ensures old clients decline rather than silently misinterpret future
 * extensions (e.g. the reserved `relay=` cross-community field).
 */
export function parseEntityLink(url: string): EntityLinkParseResult {
  let parsed: URL;
  try {
    parsed = new URL(url);
  } catch {
    return { ok: false, reason: "invalid-url" };
  }

  if (parsed.protocol !== ENTITY_LINK_SCHEME) {
    return { ok: false, reason: "wrong-scheme" };
  }

  const host = parsed.hostname;
  if (host !== "pr" && host !== "issue" && host !== "repo") {
    return { ok: false, reason: "wrong-host" };
  }

  if (parsed.username || parsed.password || parsed.port) {
    return { ok: false, reason: "invalid-structure" };
  }

  // Require empty/root path — path segments are reserved for future versioning.
  if (parsed.pathname !== "" && parsed.pathname !== "/") {
    return { ok: false, reason: "unexpected-path" };
  }

  // Reject fragments — not part of the canonical format.
  if (parsed.hash) {
    return { ok: false, reason: "unexpected-fragment" };
  }

  // Validate known params and reject unknown ones, and enforce single-instance.
  const KNOWN_REPO_PARAMS = new Set(["owner", "d", "ref", "path"]);
  const KNOWN_EVENT_PARAMS = new Set(["id", "owner", "d"]);
  const knownParams = host === "repo" ? KNOWN_REPO_PARAMS : KNOWN_EVENT_PARAMS;

  for (const key of parsed.searchParams.keys()) {
    if (!knownParams.has(key)) {
      return { ok: false, reason: "unknown-param" };
    }
  }
  for (const key of knownParams) {
    const values = parsed.searchParams.getAll(key);
    if (values.length > 1) {
      return { ok: false, reason: "duplicate-param" };
    }
  }

  const owner = parsed.searchParams.get("owner");
  const dtag = parsed.searchParams.get("d");
  if (!owner || !HEX64_RE.test(owner)) {
    return { ok: false, reason: "invalid-owner" };
  }
  if (!dtag || !isValidDtag(dtag)) {
    return { ok: false, reason: "invalid-dtag" };
  }

  if (host === "repo") {
    const hasRef = parsed.searchParams.has("ref");
    const hasPath = parsed.searchParams.has("path");
    if (hasRef !== hasPath) {
      return { ok: false, reason: "incomplete-repo-target" };
    }
    if (!hasRef) {
      return {
        ok: true,
        value: { type: "repo", owner: owner.toLowerCase(), dtag },
      };
    }

    const parsedRef = parseRepositoryRef(parsed.searchParams.get("ref") ?? "");
    if (!parsedRef) return { ok: false, reason: "invalid-ref" };
    const path = normalizeRepositoryPath(parsed.searchParams.get("path") ?? "");
    if (!path) return { ok: false, reason: "invalid-path" };
    return {
      ok: true,
      value: {
        type: "repo",
        owner: owner.toLowerCase(),
        dtag,
        target: { ref: parsedRef.value, refKind: parsedRef.kind, path },
      },
    };
  }

  const id = parsed.searchParams.get("id");
  if (!id || !HEX64_RE.test(id)) {
    return { ok: false, reason: "invalid-id" };
  }

  return {
    ok: true,
    value: {
      type: host,
      id: id.toLowerCase(),
      owner: owner.toLowerCase(),
      dtag,
    },
  };
}

/**
 * Canonical NIP-34 repository coordinate (`30617:<owner>:<dtag>`) used as the
 * route id for `/projects/$projectId`. Duncan's #4671 branch resolves 30617
 * coordinates regardless of which explicit project contains the repository, so
 * entity links remain stable when a repo's container project changes.
 *
 * Do NOT use the legacy `<owner>:<dtag>` form — it only matched implicit
 * project cards and breaks for repos claimed by an explicit project with a
 * different d-tag.
 */
export function entityLinkProjectRouteId(link: ParsedEntityLink): string {
  return `30617:${link.owner}:${link.dtag}`;
}
