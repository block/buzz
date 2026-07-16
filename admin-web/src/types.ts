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

export interface Feedback {
  id: string;
  communityId: string;
  submitterPubkey: string;
  category?: string;
  bodySummary: string;
  receivedAt: string;
}
