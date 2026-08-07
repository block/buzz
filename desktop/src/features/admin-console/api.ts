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
  | "tokenMode"
  | "disabled"
  | "notAdminApi"
  | "networkOrIntercepted";

export type AdminProbeResult = { state: AdminProbeState };

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
  status: string;
  resolvedBy?: string | null;
  resolvedAt?: string | null;
  actionId?: string | null;
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
  communityId: string;
  communityHost: string;
  eventId: string;
  submitterPubkey: string;
  category?: string | null;
  body: string;
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
  communityId: string;
  communityHost: string;
  submitterPubkey: string;
  category?: string | null;
  bodySummary: string;
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
