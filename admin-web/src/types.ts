export interface Report {
  id: string;
  communityId: string;
  communityHost: string;
  reporterPubkey: string;
  targetKind: "event" | "pubkey" | "blob";
  target: string;
  channelId?: string;
  reportType: string;
  note?: string;
  status: string;
  createdAt: string;
}

export interface ReportedMessage {
  authorPubkey: string;
  content: string;
  createdAt: string;
  deletedAt: string | null;
}

export interface ReportDetail extends Report {
  message: ReportedMessage | null;
}

export type FeedbackStatus = "new" | "reviewed" | "archived";

export interface FeedbackSummary {
  id: string;
  /// `null` once the source community is purged (provenance severed).
  communityId: string | null;
  /// `null` when `communityId` is severed — feedback retained without origin.
  communityHost: string | null;
  submitterPubkey: string;
  category?: string;
  bodySummary: string;
  status: FeedbackStatus;
  receivedAt: string;
}

export interface FeedbackDetail {
  id: string;
  /// `null` once the source community is purged (provenance severed).
  communityId: string | null;
  /// `null` when `communityId` is severed — feedback retained without origin.
  communityHost: string | null;
  eventId: string;
  submitterPubkey: string;
  category?: string;
  body: string;
  tags: string[][];
  status: FeedbackStatus;
  eventCreatedAt: string;
  receivedAt: string;
}
