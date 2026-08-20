export const BOARD_REFERENCE_TAG = "buzz:board-ref";
export const BOARD_REFERENCE_VERSION = "1";
export const MAX_BOARD_REFERENCES = 32;

export type BoardPlacement = {
  workstreamId: string;
  workstreamLabel: string;
  sectionId: string;
  sectionLabel: string;
};
export type BoardSnapshot = { label: string; detail?: string };
type Common = {
  identity: string;
  snapshot: BoardSnapshot;
  placement: BoardPlacement;
};
export type BoardReference =
  | (Common & { kind: "workstream" })
  | (Common & { kind: "pull-request" })
  | (Common & { kind: "agent" })
  | (Common & { kind: "agent-group" })
  | (Common & {
      kind: "thread-blocker";
      destination: {
        channelId: string;
        messageId: string;
        threadRootId?: string;
      };
    });

const UUID = /^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/;
const HEX = /^[0-9a-f]{64}$/;
const KINDS = new Set([
  "workstream",
  "pull-request",
  "agent",
  "agent-group",
  "thread-blocker",
]);
const CONTROL_CHARACTER = /\p{Cc}/u;
const safe = (value: unknown, max = 160): value is string =>
  typeof value === "string" &&
  value.trim().length > 0 &&
  new TextEncoder().encode(value).byteLength <= max &&
  !CONTROL_CHARACTER.test(value);

const hasExactKeys = (
  value: Record<string, unknown>,
  required: readonly string[],
  optional: readonly string[] = [],
): boolean => {
  const allowed = new Set([...required, ...optional]);
  return (
    required.every((key) => Object.hasOwn(value, key)) &&
    Object.keys(value).every((key) => allowed.has(key))
  );
};

export function isBoardReference(value: unknown): value is BoardReference {
  if (!value || typeof value !== "object" || Array.isArray(value)) return false;
  const v = value as Record<string, unknown>;
  if (typeof v.kind !== "string" || !KINDS.has(v.kind)) return false;
  const topLevelKeys =
    v.kind === "thread-blocker"
      ? ["kind", "identity", "destination", "snapshot", "placement"]
      : ["kind", "identity", "snapshot", "placement"];
  if (!hasExactKeys(v, topLevelKeys) || !safe(v.identity, 512)) return false;
  if (
    !v.snapshot ||
    typeof v.snapshot !== "object" ||
    Array.isArray(v.snapshot)
  )
    return false;
  const snapshot = v.snapshot as Record<string, unknown>;
  if (
    !hasExactKeys(snapshot, ["label"], ["detail"]) ||
    !safe(snapshot.label) ||
    (snapshot.detail !== undefined && !safe(snapshot.detail))
  )
    return false;
  const p = v.placement as Record<string, unknown> | undefined;
  if (
    !p ||
    !hasExactKeys(p, [
      "workstreamId",
      "workstreamLabel",
      "sectionId",
      "sectionLabel",
    ]) ||
    !safe(p.workstreamId, 36) ||
    !UUID.test(p.workstreamId) ||
    !safe(p.workstreamLabel) ||
    !safe(p.sectionId, 512) ||
    !safe(p.sectionLabel)
  )
    return false;
  if (v.kind === "workstream" && v.identity !== p.workstreamId) return false;
  if (v.kind === "agent" && !HEX.test(v.identity)) return false;
  if (v.kind === "thread-blocker") {
    const d = v.destination as Record<string, unknown> | undefined;
    if (
      !d ||
      !hasExactKeys(d, ["channelId", "messageId"], ["threadRootId"]) ||
      !safe(d.channelId, 36) ||
      !UUID.test(d.channelId) ||
      !safe(d.messageId, 64) ||
      !HEX.test(d.messageId) ||
      (d.threadRootId !== undefined &&
        (!safe(d.threadRootId, 64) || !HEX.test(d.threadRootId)))
    )
      return false;
  }
  return true;
}

export function boardReferenceTags(
  references: readonly BoardReference[],
): string[][] {
  if (references.length > MAX_BOARD_REFERENCES)
    throw new Error(`Select at most ${MAX_BOARD_REFERENCES} references.`);
  const seen = new Set<string>();
  return references.map((reference) => {
    if (!isBoardReference(reference) || seen.has(boardReferenceKey(reference)))
      throw new Error("Invalid or duplicate Board reference.");
    seen.add(boardReferenceKey(reference));
    const payload = JSON.stringify(reference);
    if (new TextEncoder().encode(payload).length > 4096)
      throw new Error("Board reference is too large.");
    return [BOARD_REFERENCE_TAG, BOARD_REFERENCE_VERSION, payload];
  });
}

export function parseBoardReferences(
  tags: readonly (readonly string[])[] | undefined,
  workstreamId?: string,
): BoardReference[] {
  const result: BoardReference[] = [];
  const seen = new Set<string>();
  for (const tag of tags ?? []) {
    if (result.length >= MAX_BOARD_REFERENCES) break;
    if (
      tag.length !== 3 ||
      tag[0] !== BOARD_REFERENCE_TAG ||
      tag[1] !== BOARD_REFERENCE_VERSION ||
      new TextEncoder().encode(tag[2]).length > 4096
    )
      continue;
    try {
      const value: unknown = JSON.parse(tag[2]);
      if (
        isBoardReference(value) &&
        (!workstreamId || value.placement.workstreamId === workstreamId) &&
        !seen.has(boardReferenceKey(value))
      ) {
        seen.add(boardReferenceKey(value));
        result.push(value);
      }
    } catch {
      /* malformed references do not break the message */
    }
  }
  return result;
}

export type BoardReferenceResolution = {
  reference: BoardReference;
  state: "live" | "changed" | "historical";
};
export function resolveBoardReferences(
  references: readonly BoardReference[],
  live: ReadonlyMap<string, BoardReference>,
): BoardReferenceResolution[] {
  return references.map((reference) => {
    const current = live.get(boardReferenceKey(reference));
    return {
      reference,
      state: !current
        ? "historical"
        : JSON.stringify(current.snapshot) ===
              JSON.stringify(reference.snapshot) &&
            JSON.stringify(current.placement) ===
              JSON.stringify(reference.placement)
          ? "live"
          : "changed",
    };
  });
}

export function boardReferenceKey(
  reference: Pick<BoardReference, "kind" | "identity" | "placement">,
): string {
  return `${reference.placement.workstreamId}:${reference.kind}:${reference.identity}`;
}
