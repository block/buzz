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

export interface FeedbackSummary {
  id: string;
  communityId: string;
  communityHost: string;
  submitterPubkey: string;
  category?: string;
  bodySummary: string;
  receivedAt: string;
}

export interface FeedbackDetail {
  id: string;
  communityId: string;
  communityHost: string;
  eventId: string;
  submitterPubkey: string;
  category?: string;
  body: string;
  eventCreatedAt: string;
  receivedAt: string;
}
