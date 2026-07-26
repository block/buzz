export const JOB_RESULT_SCHEMA_VERSION = 1;

export const JOB_RESULT_DISPOSITIONS = [
  "completed",
  "partial",
  "blocked",
  "failed",
  "no_artifact",
] as const;

export const JOB_ARTIFACT_KINDS = [
  "file",
  "media",
  "branch",
  "commit",
  "pull_request",
  "canvas",
  "workflow_output",
  "build",
  "deployment",
  "link",
  "other",
] as const;

export const JOB_VERIFICATION_STATUSES = [
  "passed",
  "failed",
  "not_run",
] as const;

export type JobResultDisposition = (typeof JOB_RESULT_DISPOSITIONS)[number];
export type JobArtifactKind = (typeof JOB_ARTIFACT_KINDS)[number];
export type JobVerificationStatus = (typeof JOB_VERIFICATION_STATUSES)[number];

export type JobArtifact = {
  kind: JobArtifactKind;
  label: string;
  reference: string;
  sourceState?: string;
};

export type JobVerification = {
  label: string;
  status: JobVerificationStatus;
  evidence?: string;
};

export type JobResult = {
  schemaVersion: typeof JOB_RESULT_SCHEMA_VERSION;
  jobRequest: string;
  requestedOutcome: string;
  outcome: string;
  lastProgress?: string;
  disposition: JobResultDisposition;
  artifacts: JobArtifact[];
  verification: JobVerification[];
  blocker?: string;
};

const EVENT_ID_PATTERN = /^[0-9a-f]{64}$/i;
const MAX_OUTCOME_BYTES = 8 * 1024;
const MAX_DETAIL_BYTES = 4 * 1024;
const MAX_LABEL_BYTES = 512;
const MAX_REFERENCE_BYTES = 2 * 1024;
const MAX_SOURCE_STATE_BYTES = 512;
const MAX_ITEMS = 50;
const MAX_JOB_RESULT_BYTES = 64 * 1024;

const dispositionSet = new Set<string>(JOB_RESULT_DISPOSITIONS);
const artifactKindSet = new Set<string>(JOB_ARTIFACT_KINDS);
const verificationStatusSet = new Set<string>(JOB_VERIFICATION_STATUSES);
const textEncoder = new TextEncoder();
const supportedReferenceSchemes = new Set([
  "http:",
  "https:",
  "buzz:",
  "nostr:",
]);

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

function readText(
  value: unknown,
  maxBytes: number,
  options: { singleLine?: boolean } = {},
): string | null {
  if (typeof value !== "string") {
    return null;
  }

  const trimmed = value.trim();
  if (trimmed.length === 0 || textEncoder.encode(value).length > maxBytes) {
    return null;
  }

  if (
    [...trimmed].some((character) => {
      const codePoint = character.codePointAt(0) ?? 0;
      const isControl =
        codePoint < 32 || (codePoint >= 127 && codePoint <= 159);
      return (
        isControl &&
        (options.singleLine || (character !== "\n" && character !== "\t"))
      );
    })
  ) {
    return null;
  }

  return trimmed;
}

function readOptionalText(
  value: unknown,
  maxBytes: number,
  options: { singleLine?: boolean } = {},
): string | null | undefined {
  if (value === undefined || value === null) {
    return undefined;
  }

  return readText(value, maxBytes, options);
}

function validateReferenceUrl(value: string): boolean | null {
  let url: URL;
  try {
    url = new URL(value);
  } catch {
    return null;
  }

  return (
    supportedReferenceSchemes.has(url.protocol) &&
    (!["http:", "https:"].includes(url.protocol) || Boolean(url.hostname)) &&
    url.username.length === 0 &&
    url.password.length === 0
  );
}

function isValidArtifactReference(
  kind: JobArtifactKind,
  reference: string,
): boolean {
  const urlValidity = validateReferenceUrl(reference);
  if (urlValidity === false) {
    return false;
  }

  switch (kind) {
    case "pull_request":
    case "build":
    case "deployment":
    case "link":
    case "media":
      return urlValidity === true;
    case "file":
      if (urlValidity !== null) {
        return urlValidity;
      }
      return (
        !reference.startsWith("/") &&
        !reference.startsWith("\\") &&
        !reference.startsWith("~/") &&
        !reference.startsWith("~\\") &&
        !/^[A-Za-z]:/.test(reference) &&
        !reference.split(/[\\/]/).includes("..")
      );
    case "commit":
      if (urlValidity !== null) {
        return urlValidity;
      }
      return (
        [40, 64].includes(reference.length) && /^[0-9a-f]+$/i.test(reference)
      );
    case "branch":
    case "canvas":
    case "workflow_output":
    case "other":
      return true;
  }
}

function parseArtifact(value: unknown): JobArtifact | null {
  if (!isRecord(value) || !artifactKindSet.has(String(value.kind))) {
    return null;
  }

  const label = readText(value.label, MAX_LABEL_BYTES, { singleLine: true });
  const reference = readText(value.reference, MAX_REFERENCE_BYTES, {
    singleLine: true,
  });
  const sourceState = readOptionalText(
    value.sourceState,
    MAX_SOURCE_STATE_BYTES,
    { singleLine: true },
  );
  if (
    !label ||
    !reference ||
    sourceState === null ||
    !isValidArtifactReference(value.kind as JobArtifactKind, reference)
  ) {
    return null;
  }

  return {
    kind: value.kind as JobArtifactKind,
    label,
    reference,
    ...(sourceState ? { sourceState } : {}),
  };
}

function parseVerification(value: unknown): JobVerification | null {
  if (!isRecord(value) || !verificationStatusSet.has(String(value.status))) {
    return null;
  }

  const label = readText(value.label, MAX_LABEL_BYTES, { singleLine: true });
  const evidence = readOptionalText(value.evidence, MAX_REFERENCE_BYTES);
  if (!label || evidence === null) {
    return null;
  }

  return {
    label,
    status: value.status as JobVerificationStatus,
    ...(evidence ? { evidence } : {}),
  };
}

function parseArray<T>(
  value: unknown,
  parseItem: (item: unknown) => T | null,
): T[] | null {
  if (value === undefined) {
    return [];
  }
  if (!Array.isArray(value) || value.length > MAX_ITEMS) {
    return null;
  }

  const parsed = value.map(parseItem);
  return parsed.every((item): item is T => item !== null) ? parsed : null;
}

export function getJobResultRequestId(tags: string[][]): string | null {
  const replyTags = tags.filter((tag) => tag[0] === "e" && tag[3] === "reply");
  if (replyTags.length !== 1) {
    return null;
  }

  const eventId = replyTags[0]?.[1];
  return eventId && EVENT_ID_PATTERN.test(eventId) ? eventId : null;
}

/**
 * Parse the versioned `kind:43004` payload.
 *
 * Invalid, legacy, and unsupported content returns null so callers can fall
 * back to the existing Markdown renderer without trusting partial fields.
 */
export function parseJobResultContent(
  content: string,
  expectedJobRequest: string | null,
): JobResult | null {
  if (
    textEncoder.encode(content).length > MAX_JOB_RESULT_BYTES ||
    !expectedJobRequest ||
    !EVENT_ID_PATTERN.test(expectedJobRequest)
  ) {
    return null;
  }

  let value: unknown;
  try {
    value = JSON.parse(content);
  } catch {
    return null;
  }

  if (
    !isRecord(value) ||
    value.schemaVersion !== JOB_RESULT_SCHEMA_VERSION ||
    !EVENT_ID_PATTERN.test(String(value.jobRequest)) ||
    String(value.jobRequest).toLowerCase() !==
      expectedJobRequest.toLowerCase() ||
    !dispositionSet.has(String(value.disposition))
  ) {
    return null;
  }

  const requestedOutcome = readText(value.requestedOutcome, MAX_OUTCOME_BYTES);
  const outcome = readText(value.outcome, MAX_OUTCOME_BYTES);
  const lastProgress = readOptionalText(value.lastProgress, MAX_DETAIL_BYTES);
  const blocker = readOptionalText(value.blocker, MAX_DETAIL_BYTES);
  const artifacts = parseArray(value.artifacts, parseArtifact);
  const verification = parseArray(value.verification, parseVerification);

  if (
    !requestedOutcome ||
    !outcome ||
    lastProgress === null ||
    blocker === null ||
    !artifacts ||
    !verification
  ) {
    return null;
  }

  const disposition = value.disposition as JobResultDisposition;
  if (
    (disposition === "completed" && artifacts.length === 0) ||
    (disposition === "no_artifact" && artifacts.length > 0) ||
    (disposition === "blocked" && !blocker)
  ) {
    return null;
  }

  return {
    schemaVersion: JOB_RESULT_SCHEMA_VERSION,
    jobRequest: String(value.jobRequest).toLowerCase(),
    requestedOutcome,
    outcome,
    lastProgress,
    disposition,
    artifacts,
    verification,
    blocker,
  };
}

export function getJobResultDispositionLabel(
  disposition: JobResultDisposition,
): string {
  switch (disposition) {
    case "completed":
      return "Completed";
    case "partial":
      return "Partially completed";
    case "blocked":
      return "Blocked";
    case "failed":
      return "Failed";
    case "no_artifact":
      return "Completed without an artifact";
  }
}

export function getJobResultFeedHeadline(
  disposition: JobResultDisposition,
): string {
  switch (disposition) {
    case "completed":
    case "no_artifact":
      return "Job completed";
    case "partial":
      return "Job partially completed";
    case "blocked":
      return "Job blocked";
    case "failed":
      return "Job failed";
  }
}

export function getJobResultFeedPresentation(
  content: string,
  expectedJobRequest: string | null,
): { headline: string; content: string } | null {
  const result = parseJobResultContent(content, expectedJobRequest);
  if (!result) {
    return null;
  }

  return {
    headline: getJobResultFeedHeadline(result.disposition),
    content: result.outcome,
  };
}

export function getJobArtifactKindLabel(kind: JobArtifactKind): string {
  switch (kind) {
    case "file":
      return "File";
    case "media":
      return "Media";
    case "branch":
      return "Branch";
    case "commit":
      return "Commit";
    case "pull_request":
      return "Pull request";
    case "canvas":
      return "Canvas";
    case "workflow_output":
      return "Workflow output";
    case "build":
      return "Build";
    case "deployment":
      return "Deployment";
    case "link":
      return "Link";
    case "other":
      return "Artifact";
  }
}
