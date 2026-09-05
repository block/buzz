/**
 * TypeScript wrappers for the desktop admin console Tauri commands.
 *
 * All network activity is native (Rust). The webview never constructs
 * admin API URLs — it supplies typed arguments which the Rust layer maps
 * to the closed route enum.
 *
 * State keying: every result is implicitly tied to `(activePubkey, origin)`.
 * Callers must cancel in-flight queries on pubkey or origin change.
 */

import { invokeTauri } from "@/shared/api/tauri";
import { invoke as invokeTauriRaw } from "@tauri-apps/api/core";

// ── Probe ─────────────────────────────────────────────────────────────────

/**
 * Result of probing an admin origin. Each variant drives a distinct settings
 * UI state. See `AdminProbeResult` in the Rust module for the full contract.
 */
export type AdminProbeState =
  | "nip98Authorized"
  | "nip98Denied"
  | "disabled"
  | "notAdminApi"
  | "networkOrIntercepted";

/**
 * The resolved principal role, present only in `nip98Authorized` state.
 * Matches the relay's `operator|moderator` vocabulary.
 */
export type AdminPrincipalRole = "operator" | "moderator";

/**
 * How the principal's role was resolved — determines whether staffing
 * controls are editable in the UI.
 */
export type AdminPrincipalSource = "config" | "owner_fallback" | "db";

export type AdminProbeResult = {
  state: AdminProbeState;
  /** Present when state is `nip98Authorized`. */
  role?: AdminPrincipalRole | null;
  /** Present when state is `nip98Authorized`. */
  source?: AdminPrincipalSource | null;
};

/**
 * Probe `origin` to determine the authentication mode and whether the current
 * app keypair is authorised.
 *
 * Returns `nip98Authorized` only on a fully authenticated 2xx. All other
 * states map directly to informational UI copy without further retries.
 */
export async function probeAdminOrigin(
  origin: string,
): Promise<AdminProbeResult> {
  return invokeTauri<AdminProbeResult>("admin_probe", { origin });
}

// ── Origin persistence ────────────────────────────────────────────────────

/**
 * Return the saved admin console origin for the currently active pubkey, or
 * `null` if none has been saved.
 *
 * `expectedPubkey` is forwarded to the Rust command as a defence-in-depth
 * guard: if the active signing key no longer matches the pubkey that was
 * active when the call was issued (delayed IPC after an identity switch), the
 * Rust side rejects the read. Callers should pass the pubkey that was active
 * when the request was initiated.
 */
export async function getAdminOrigin(
  expectedPubkey?: string,
): Promise<string | null> {
  return invokeTauri<string | null>("get_admin_origin", { expectedPubkey });
}

/**
 * Validate, normalise, and save `rawOrigin` as the admin console origin for
 * the current pubkey. Returns the canonical origin on success.
 * Pass `null` to clear the saved origin.
 *
 * `expectedPubkey` is forwarded to the Rust command: if the active signing
 * key no longer matches, the write is rejected so a delayed save cannot write
 * identity A's input into identity B's storage namespace.
 */
export async function setAdminOrigin(
  rawOrigin: string | null,
  expectedPubkey?: string,
): Promise<string | null> {
  return invokeTauri<string | null>("set_admin_origin", {
    rawOrigin,
    expectedPubkey,
  });
}

/**
 * Auto-discover the admin console origin from the connected relay's NIP-11
 * document (`admin_api` field). Returns the canonical origin when the relay
 * advertises a valid one, or `null` when it does not — the caller falls back
 * to manual entry. Rejects only on a transport or relay error; an absent or
 * invalid advertised value resolves to `null`, never throws.
 */
export async function discoverAdminOrigin(): Promise<string | null> {
  return invokeTauri<string | null>("admin_discover_origin");
}

// ── Wire DTO types ────────────────────────────────────────────────────────
//
// Mirror `crates/buzz-db/src/admin_moderation.rs` field-for-field.
// Rust structs use `#[serde(rename_all = "camelCase")]`; DateTime<Utc>
// serialises to an ISO-8601 string; Option<T> serialises to null / absent.

/** Deployment-global moderation report (list and detail base). */
export type AdminReportDto = {
  id: string;
  communityId: string;
  communityHost: string;
  reportEventId: string;
  reporterPubkey: string;
  targetKind: string;
  target: string;
  channelId?: string | null;
  reportType: string;
  note?: string | null;
  /**
   * Report status. Values: `open` | `processing` | `resolved` | `dismissed` | `escalated`.
   * A `processing` report has an in-progress enforcement action; it must NOT be
   * presented as actionable in the UI.
   */
  status: string;
  resolvedBy?: string | null;
  resolvedAt?: string | null;
  actionId?: string | null;
  /**
   * Present when status is `processing` or the report has an active/failed action.
   * Drives the enforcement-state rendering.
   */
  activeAction?: AdminActionRecordDto | null;
  createdAt: string;
};

/** Reported message snapshot attached to an AdminReportDetail. */
export type AdminReportedMessageDto = {
  authorPubkey: string;
  content: string;
  createdAt: string;
  deletedAt?: string | null;
};

/**
 * Full report detail — AdminReport fields flattened with an optional
 * nested message (present when the report targets a stored event).
 */
export type AdminReportDetailDto = AdminReportDto & {
  message?: AdminReportedMessageDto | null;
};

/** Deployment-global product feedback entry. */
export type AdminFeedbackDto = {
  id: string;
  /**
   * Source community. Both are `null` once the source community has been
   * purged: feedback is deployment-global operator evidence whose
   * `communityId` is severed to NULL on tenant purge, not cascade-deleted.
   */
  communityId: string | null;
  communityHost: string | null;
  eventId: string;
  submitterPubkey: string;
  category?: string | null;
  body: string;
  /** Triage status: `"new"` | `"reviewed"` | `"archived"`. Always present. */
  status: AdminFeedbackStatus;
  /** Full source tags — consumed as imeta attachment metadata. */
  tags: unknown;
  eventCreatedAt: string;
  receivedAt: string;
};

/**
 * Feedback list row returned by the relay's `GET /admin/feedback` handler.
 * Authoritative source: `buzz-relay/src/api/admin/mod.rs` `FeedbackSummary`.
 *
 * This is a separate, leaner shape from `AdminFeedbackDto` — the list
 * endpoint summarises the body and omits event/tag detail fields that are
 * only needed when viewing a single entry.
 */
export type AdminFeedbackSummaryDto = {
  id: string;
  /** Source community — `null` on a severed (purged-source) row. */
  communityId: string | null;
  communityHost: string | null;
  submitterPubkey: string;
  category?: string | null;
  bodySummary: string;
  /** Triage status: `"new"` | `"reviewed"` | `"archived"`. Always present. */
  status: AdminFeedbackStatus;
  receivedAt: string;
};

// ── Data commands ─────────────────────────────────────────────────────────

export type AdminReportsQuery = {
  communityId?: string;
  status?: string;
  reportType?: string;
  targetKind?: string;
  after?: string;
  before?: string;
  limit?: number;
  /**
   * Visibility scope for the reports list.
   * - Omitted (default): relay returns escalated-only — the platform-safety
   *   backstop queue for callers that want the narrow view.
   * - `"all"`: relay returns every status (`open`, `processing`, `resolved`,
   *   `dismissed`, `escalated`). The admin console always requests `"all"` so
   *   operators can see and act on the full workflow queue.
   *
   * Ignored by the relay when an explicit `status` filter is present.
   */
  scope?: "all";
};

/** Fetch the deployment-wide reports list. */
export async function listAdminReports(
  origin: string,
  query: AdminReportsQuery = {},
): Promise<AdminReportDto[]> {
  return invokeTauri<AdminReportDto[]>("admin_list_reports", { origin, query });
}

/** Fetch a single report's detail by ID. */
export async function getAdminReport(
  origin: string,
  id: string,
): Promise<AdminReportDetailDto> {
  return invokeTauri<AdminReportDetailDto>("admin_get_report", { origin, id });
}

/** Fetch the deployment-wide product feedback list. */
export async function listAdminFeedback(
  origin: string,
): Promise<AdminFeedbackSummaryDto[]> {
  return invokeTauri<AdminFeedbackSummaryDto[]>("admin_list_feedback", {
    origin,
  });
}

/** Fetch a single feedback entry's detail (includes imeta attachment metadata). */
export async function getAdminFeedback(
  origin: string,
  id: string,
): Promise<AdminFeedbackDto> {
  return invokeTauri<AdminFeedbackDto>("admin_get_feedback", { origin, id });
}

// ── Actions ───────────────────────────────────────────────────────────────

/**
 * Valid actions per target_kind (v4 §7 frozen matrix).
 *
 * event:  delete | kick | ban | timeout | dismiss | escalate
 * pubkey: ban | timeout | dismiss | escalate
 * blob:   dismiss | escalate
 */
export type AdminReportAction =
  | "delete"
  | "kick"
  | "ban"
  | "timeout"
  | "dismiss"
  | "escalate";

/**
 * Body for POST /api/admin/v1/reports/{id}/resolve.
 *
 * `requestId` is a client-generated UUID. Generate once per resolution
 * attempt and **reuse on retry after a lost response** (v4 amendment 2).
 *
 * `expirationSecs` is required for `timeout` and must be omitted otherwise.
 */
export type AdminResolveReportBody = {
  action: AdminReportAction;
  requestId: string;
  expirationSecs?: number;
  reason?: string;
};

/**
 * The action record returned in the resolve response (or from the report detail
 * when status is `processing`/`failed`).
 *
 * Field-for-field the relay's serialized action record. In practice `action` is
 * always an enforcement action (`delete`/`kick`/`ban`/`timeout`) — `dismiss` and
 * `escalate` terminalise the report without creating a record, so they surface as
 * `activeAction: null`, never here.
 *
 * `expiresAt` is the absolute enforcement expiry (`timeout_until`), null except for
 * `timeout`. It is distinct from the resolve request's `expirationSecs` input.
 */
export type AdminActionRecordDto = {
  id: string;
  requestId: string;
  actorPubkey: string;
  actorRole: AdminPrincipalRole;
  action: AdminReportAction;
  status: "pending" | "enforcing" | "succeeded" | "failed" | "cancelled";
  reason: string | null;
  expiresAt: string | null;
  errorMessage: string | null;
  createdAt: string;
  updatedAt: string;
};

/**
 * Uniform envelope returned by resolve and cancel: the report's new terminal
 * status plus the governing action record. `activeAction` is null for
 * decision-only resolutions (`dismiss`/`escalate`, which create no record).
 *
 * Both endpoints re-read the report so this shape matches a subsequent
 * `GET /reports/{id}` — the console reloads detail after a mutation rather than
 * consuming this body, so it is a wire contract, not a render source.
 */
export type AdminReportResolution = {
  status: string;
  activeAction: AdminActionRecordDto | null;
};

/**
 * Resolve a report — POST /api/admin/v1/reports/{id}/resolve.
 *
 * The caller must generate a UUID `requestId` per resolution attempt and
 * reuse the **same** UUID on retry after a lost response. A different
 * `requestId` against a `processing` report yields 409.
 */
export async function resolveAdminReport(
  origin: string,
  id: string,
  body: AdminResolveReportBody,
): Promise<AdminReportResolution> {
  return invokeTauri<AdminReportResolution>("admin_resolve_report", {
    origin,
    id,
    body,
  });
}

/**
 * Body for POST /api/admin/v1/reports/{id}/cancel.
 *
 * `actionId` fences the cancel to exactly the failed action the operator
 * observed. A mismatch — already cancelled, superseded by a newer claim, or
 * past the mutation point — resolves to 409.
 */
export type AdminCancelReportBody = {
  actionId: string;
};

/**
 * Cancel a failed enforcement action — POST /api/admin/v1/reports/{id}/cancel.
 *
 * The only recovery path for a `failed` action: it returns the report to
 * `open` for a fresh resolution attempt (there is no composed client-side
 * retry — that would imply an atomicity the relay does not provide). A `409`
 * means the action is no longer cancellable; treat it as "refresh detail" —
 * someone else likely cancelled it or it advanced past the mutation point.
 *
 * The response embeds the just-cancelled action as a last look; a subsequent
 * detail read serves `activeAction: null`.
 */
export async function cancelAdminReport(
  origin: string,
  id: string,
  body: AdminCancelReportBody,
): Promise<AdminReportResolution> {
  return invokeTauri<AdminReportResolution>("admin_cancel_report", {
    origin,
    id,
    body,
  });
}

/**
 * Body for POST /api/admin/v1/reports/{id}/reopen.
 *
 * `requestId` is a client-generated UUID; generate once per reopen attempt and
 * reuse the **same** UUID on retry after a lost response, mirroring resolve
 * idempotency.
 */
export type AdminReopenReportBody = {
  requestId: string;
  reason?: string;
};

/** The status returned by a successful reopen — always `"open"`. */
export type AdminReopenReportResult = {
  status: string;
};

/**
 * Reopen a resolved report — POST /api/admin/v1/reports/{id}/reopen.
 *
 * Moves a `resolved` | `dismissed` | `escalated` report back to `open` for
 * re-triage. A `processing` report is not reopenable and yields 409. Reopen
 * does **not** reverse enforcement (no un-ban, no un-delete) — it only
 * re-queues the report.
 *
 * The caller must generate a UUID `requestId` per reopen attempt and reuse the
 * **same** UUID on retry after a lost response.
 */
export async function reopenAdminReport(
  origin: string,
  id: string,
  body: AdminReopenReportBody,
): Promise<AdminReopenReportResult> {
  return invokeTauri<AdminReopenReportResult>("admin_reopen_report", {
    origin,
    id,
    body,
  });
}

// ── Feedback status ───────────────────────────────────────────────────────

export type AdminFeedbackStatus = "new" | "reviewed" | "archived";

/**
 * The PATCH /api/admin/v1/feedback/{id} response — the relay echoes only the
 * updated `status`, not a full feedback record.
 */
export type AdminFeedbackStatusResult = {
  status: AdminFeedbackStatus;
};

/** Update feedback status — PATCH /api/admin/v1/feedback/{id}. */
export async function patchAdminFeedback(
  origin: string,
  id: string,
  status: AdminFeedbackStatus,
): Promise<AdminFeedbackStatusResult> {
  return invokeTauri<AdminFeedbackStatusResult>("admin_patch_feedback", {
    origin,
    id,
    body: { status },
  });
}

// ── Staffing ──────────────────────────────────────────────────────────────

/**
 * An effective principal entry returned by GET /api/admin/v1/operators.
 * `effectiveRole` is the resolved role; `sources` explains where it comes from.
 */
export type AdminOperatorDto = {
  pubkey: string;
  effectiveRole: "operator" | "moderator";
  sources: Array<"config" | "owner_fallback" | "db">;
};

/** List all effective principals — GET /api/admin/v1/operators. Operator-only. */
export async function listAdminOperators(
  origin: string,
): Promise<AdminOperatorDto[]> {
  return invokeTauri<AdminOperatorDto[]>("admin_list_operators", { origin });
}

/**
 * Add or update an operator — PUT /api/admin/v1/operators/{pubkey}.
 * Returns 409 (as a thrown error string) if the pubkey is config-backed.
 */
export async function putAdminOperator(
  origin: string,
  pubkey: string,
  role: "operator" | "moderator",
): Promise<AdminOperatorDto> {
  return invokeTauri<AdminOperatorDto>("admin_put_operator", {
    origin,
    pubkey,
    body: { role },
  });
}

/**
 * Remove an operator — DELETE /api/admin/v1/operators/{pubkey}.
 * Returns 409 (as a thrown error string) if the pubkey is config-backed.
 */
export async function deleteAdminOperator(
  origin: string,
  pubkey: string,
): Promise<void> {
  return invokeTauri<void>("admin_delete_operator", { origin, pubkey });
}

// ── Attachment ────────────────────────────────────────────────────────────

/**
 * Stable typed error codes returned by `admin_fetch_feedback_attachment`.
 * These map to actionable UI states — never silently ignored.
 */
export type AdminAttachmentErrorCode =
  | "admin_attachment_too_large"
  | "admin_attachment_mime_mismatch"
  | "admin_attachment_size_mismatch"
  | "admin_attachment_invalid_hash"
  | "admin_attachment_invalid_mime"
  | "admin_attachment_invalid_size"
  | "admin_attachment_network_error"
  | "admin_attachment_redirect"
  | string; // relay HTTP error codes like admin_attachment_relay_error_404

/**
 * Fetch a feedback attachment as raw bytes, then construct a Blob URL.
 *
 * The caller MUST supply `expectedMime` and `expectedSize` from the
 * server-validated `imeta` fields in the feedback detail response. The native
 * layer validates the relay's `Content-Type` and byte count against these
 * expected values before returning; a mismatch yields a typed error code.
 *
 * The Blob is constructed from `expectedMime` — never a response header —
 * so MIME is anchored to the server-validated imeta metadata.
 *
 * **Callers must `URL.revokeObjectURL(url)` when the URL is no longer needed.**
 *
 * @returns A `blob:` URL on success.
 * @throws The typed error code string on failure.
 */
export async function fetchAdminAttachmentBlobUrl(
  origin: string,
  feedbackId: string,
  sha256: string,
  expectedMime: string,
  expectedSize: number,
): Promise<string> {
  // The Rust command returns `tauri::ipc::Response` — arrives as ArrayBuffer.
  const buffer = await invokeTauriRaw<ArrayBuffer>(
    "admin_fetch_feedback_attachment",
    {
      origin,
      feedbackId,
      sha256,
      expectedMime,
      expectedSize,
    },
  );
  const blob = new Blob([buffer], { type: expectedMime });
  return URL.createObjectURL(blob);
}
