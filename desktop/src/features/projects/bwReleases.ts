import type { RelayEvent } from "@/shared/api/types";
import type { BwSnapshot } from "./bwProjection";

// P5_6A: read-only release/run/artifact views over the accepted Core
// projection. Presentation only — like `mergeBwIssues`, nothing here decides
// acceptance, completion or evidence: set status comes verbatim from
// `projection.sets`, the current handoff from `projection.handoffs`, member
// verdicts from `projection.artifact_verdicts`, details from Core-accepted
// `records`, and refused/pending chains from `decisions`/`conflicts`. A
// provider run success never upgrades anything to `completed` here.

export type BwPipelineView = {
  id: string;
  workflow: string | null;
  workflowSha256: string | null;
  provider: string | null;
  runner: string | null;
  mirror: string | null;
};

export type BwReleaseMemberView = {
  issue: string;
  /** The implemented-transition event the freeze bound for this member. */
  implemented: string | null;
  /** Current Core issue state — historical acceptance lives here, separate
   * from any artifact verdict. */
  state: string | null;
  title: string | null;
};

export type BwRequestView = {
  id: string;
  worker: string | null;
  attempt: number | null;
  retryOf: string | null;
};

export type BwRunView = {
  id: string;
  request: string | null;
  provider: string | null;
  runId: string | null;
  attempt: number | null;
  url: string | null;
  head: string | null;
  /** success | failure | cancelled — the provider result, never a verdict. */
  result: string | null;
};

export type BwArtifactView = {
  id: string;
  mime: string | null;
  bytes: number | null;
  sha256: string | null;
  url: string | null;
  /** Core's per-member verdicts for exactly this artifact
   * (accepted | rejected | unreviewed | conflict). */
  verdicts: Record<string, string>;
};

export type BwHandoffView = {
  id: string;
  run: string | null;
  artifacts: string[];
  tester: string | null;
  limitations: string | null;
  installation: {
    method: string | null;
    url: string | null;
    instructions: string | null;
    unsigned: boolean;
    immutable: boolean;
  } | null;
};

export type BwReleaseSetView = {
  id: string;
  release: string | null;
  platform: string | null;
  stream: string | null;
  /** The frozen canonical head (`relay_sha`) the set was cut at. */
  freezeSha: string | null;
  frozenAt: number | null;
  /** `projection.sets` verbatim: active | completed | failed | aborted. */
  status: string;
  pipeline: BwPipelineView | null;
  members: BwReleaseMemberView[];
  requests: BwRequestView[];
  runs: BwRunView[];
  artifacts: BwArtifactView[];
  /** The current handoff Core projects for this set, if any was accepted. */
  handoff: BwHandoffView | null;
};

export type BwIncompleteEvidence = {
  eventId: string;
  /** pending | reject | conflict — a non-accepted Core decision. */
  outcome: string;
  stage: string;
  code: string;
};

export type BwReleasesView = {
  sets: BwReleaseSetView[];
  /** Every non-accepted decision in the snapshot, as traceable references. */
  incomplete: BwIncompleteEvidence[];
  conflicts: string[];
  building: boolean;
  hostAuthorized: boolean;
  dispatchCount: number;
};

function tag(event: RelayEvent, name: string): string | undefined {
  return event.tags.find((entry) => entry[0] === name)?.[1];
}

function recordType(event: RelayEvent): string | null {
  return event.kind === 46100 ? (tag(event, "record") ?? null) : null;
}

function bodyOf(event: RelayEvent): Record<string, unknown> {
  try {
    const parsed: unknown = JSON.parse(event.content);
    return typeof parsed === "object" &&
      parsed !== null &&
      !Array.isArray(parsed)
      ? (parsed as Record<string, unknown>)
      : {};
  } catch {
    return {};
  }
}

function asString(value: unknown): string | null {
  return typeof value === "string" ? value : null;
}

function asNumber(value: unknown): number | null {
  return typeof value === "number" && Number.isFinite(value) ? value : null;
}

function asBoolean(value: unknown): boolean {
  return value === true;
}

function acceptedOfType(snapshot: BwSnapshot, type: string): RelayEvent[] {
  return Object.values(snapshot.records ?? {}).filter(
    (event) => recordType(event) === type,
  );
}

function byTimeThenId(left: RelayEvent, right: RelayEvent): number {
  return (
    (left.created_at ?? 0) - (right.created_at ?? 0) ||
    left.id.localeCompare(right.id)
  );
}

function pipelineView(
  snapshot: BwSnapshot,
  pipelineId: string | null,
): BwPipelineView | null {
  if (!pipelineId) return null;
  const record = snapshot.records?.[pipelineId];
  if (!record || recordType(record) !== "release-pipeline") return null;
  const body = bodyOf(record);
  return {
    id: pipelineId,
    workflow: asString(body.workflow),
    workflowSha256: asString(body.workflow_sha256),
    provider: asString(body.provider),
    runner: asString(body.runner),
    mirror: asString(body.mirror),
  };
}

function handoffView(
  snapshot: BwSnapshot,
  handoffId: string | undefined,
): BwHandoffView | null {
  if (!handoffId) return null;
  const record = snapshot.records?.[handoffId];
  if (!record || recordType(record) !== "test-ready") return null;
  const body = bodyOf(record);
  const installation =
    typeof body.installation === "object" && body.installation !== null
      ? (body.installation as Record<string, unknown>)
      : null;
  return {
    id: handoffId,
    run: asString(body.run),
    artifacts: Array.isArray(body.artifacts)
      ? body.artifacts.filter((id): id is string => typeof id === "string")
      : [],
    tester: asString(body.tester),
    limitations: asString(body.limitations),
    installation: installation
      ? {
          method: asString(installation.method),
          url: asString(installation.url),
          instructions: asString(installation.instructions),
          unsigned: asBoolean(installation.unsigned),
          immutable: asBoolean(installation.immutable),
        }
      : null,
  };
}

function releaseSetView(
  snapshot: BwSnapshot,
  setId: string,
  status: string,
): BwReleaseSetView {
  const projection = snapshot.projection;
  const record = snapshot.records?.[setId];
  const frozen = record ? bodyOf(record) : {};
  const members = (Array.isArray(frozen.members) ? frozen.members : [])
    .filter(
      (member): member is Record<string, unknown> =>
        typeof member === "object" && member !== null,
    )
    .map((member) => {
      const issue = asString(member.issue) ?? "";
      return {
        issue,
        implemented: asString(member.implemented),
        state: projection.issues?.[issue] ?? null,
        title: projection.issue_fields?.[issue]?.title ?? null,
      };
    });
  const requests = acceptedOfType(snapshot, "build-request")
    .filter((event) => asString(bodyOf(event).set) === setId)
    .sort(byTimeThenId)
    .map((event) => {
      const body = bodyOf(event);
      return {
        id: event.id,
        worker: asString(body.worker),
        attempt: asNumber(body.attempt),
        retryOf: asString(body.retry_of),
      };
    });
  const requestIds = new Set(requests.map((request) => request.id));
  const runs = acceptedOfType(snapshot, "build-run")
    .filter((event) => requestIds.has(asString(bodyOf(event).request) ?? ""))
    .sort(byTimeThenId)
    .map((event) => {
      const body = bodyOf(event);
      return {
        id: event.id,
        request: asString(body.request),
        provider: asString(body.provider),
        runId: asString(body.run_id),
        attempt: asNumber(body.attempt),
        url: asString(body.url),
        head: asString(body.head),
        result: asString(body.result),
      };
    });
  const artifacts = Object.values(snapshot.records ?? {})
    .filter((event) => event.kind === 1063 && tag(event, "set") === setId)
    .sort(byTimeThenId)
    .map((event) => ({
      id: event.id,
      mime: tag(event, "m") ?? null,
      bytes: Number.isFinite(Number(tag(event, "size")))
        ? Number(tag(event, "size"))
        : null,
      sha256: tag(event, "x") ?? null,
      url: tag(event, "url") ?? null,
      verdicts: projection.artifact_verdicts?.[event.id] ?? {},
    }));
  return {
    id: setId,
    release: asString(frozen.release),
    platform: asString(frozen.platform),
    stream: asString(frozen.stream),
    freezeSha: asString(frozen.relay_sha),
    frozenAt: record?.created_at ?? null,
    status,
    pipeline: pipelineView(snapshot, asString(frozen.pipeline)),
    members,
    requests,
    runs,
    artifacts,
    handoff: handoffView(snapshot, projection.handoffs?.[setId]),
  };
}

/** The complete read-only release view for one repository snapshot. Sets are
 * ordered by freeze time; every status/verdict is Core's, every detail comes
 * from Core-accepted records, and every non-accepted decision stays visible
 * as a traceable reference. */
export function bwReleasesView(snapshot: BwSnapshot): BwReleasesView {
  const projection = snapshot.projection;
  const statuses = projection.sets ?? {};
  const records = snapshot.records ?? {};
  const setIds = Object.keys(statuses).sort((left, right) => {
    const leftAt = records[left]?.created_at ?? Number.MAX_SAFE_INTEGER;
    const rightAt = records[right]?.created_at ?? Number.MAX_SAFE_INTEGER;
    return leftAt - rightAt || left.localeCompare(right);
  });
  const incomplete = Object.entries(snapshot.decisions ?? {})
    .filter(([, decision]) => decision.outcome !== "accept")
    .map(([eventId, decision]) => ({
      eventId,
      outcome: decision.outcome,
      stage: decision.stage,
      code: decision.code,
    }))
    .sort((left, right) => left.eventId.localeCompare(right.eventId));
  return {
    sets: setIds.map((id) => releaseSetView(snapshot, id, statuses[id])),
    incomplete,
    conflicts: projection.conflicts ?? [],
    building: projection.building === true,
    hostAuthorized: projection.host_authorized === true,
    dispatchCount: projection.dispatch_count ?? 0,
  };
}

const EVIDENCE_REASONS: Record<string, string> = {
  "relay-head": "Awaiting canonical Git head observation",
  "git-ancestry": "Awaiting Git ancestry observation",
  "provider-readback": "Awaiting CI provider readback",
  "artifact-download": "Awaiting artifact download evidence",
  "artifact-digest": "Download digest mismatch",
  durability: "Download not durable or immutable",
  "verdict-conflict": "Conflicting member verdicts",
  "missing-reference": "Awaiting a referenced event",
};

/** Human-readable reason for a pending/refused/conflicted decision. Missing
 * external evidence stays honestly pending — the label never claims success
 * and unknown codes fall back to the raw Core stage/code. */
export function bwEvidenceReasonLabel(
  entry: Pick<BwIncompleteEvidence, "stage" | "code">,
): string {
  return EVIDENCE_REASONS[entry.code] ?? `${entry.stage}: ${entry.code}`;
}
