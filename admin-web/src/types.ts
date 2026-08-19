export interface Report {
  id: string;
  communityId: string;
  communityHost: string;
  /** Legacy protocol-hex identity. Prefer reporterNpub when present. */
  reporterPubkey: string;
  reporterNpub?: string;
  targetKind: "event" | "pubkey" | "blob";
  target: string;
  targetNpub?: string;
  channelId?: string;
  reportType: string;
  note?: string;
  status: string;
  resolvedBy?: string | null;
  resolvedByNpub?: string | null;
  createdAt: string;
}

export interface ReportedMessage {
  /** Legacy protocol-hex identity. Prefer authorNpub when present. */
  authorPubkey: string;
  authorNpub?: string;
  content: string;
  createdAt: string;
  deletedAt: string | null;
}

export interface ReportDetail extends Report {
  message: ReportedMessage | null;
}

export interface FeedbackSummary {
  id: string;
  communityId: string;
  communityHost: string;
  /** Legacy protocol-hex identity. Prefer submitterNpub when present. */
  submitterPubkey: string;
  submitterNpub?: string;
  category?: string;
  bodySummary: string;
  receivedAt: string;
}

export interface FeedbackDetail {
  id: string;
  communityId: string;
  communityHost: string;
  eventId: string;
  /** Legacy protocol-hex identity. Prefer submitterNpub when present. */
  submitterPubkey: string;
  submitterNpub?: string;
  category?: string;
  body: string;
  tags: string[][];
  eventCreatedAt: string;
  receivedAt: string;
}
