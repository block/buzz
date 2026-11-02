import type { ProspectStage } from "./businessRecordContract";
import type { BusinessRecords, BusinessRole } from "./businessRecordRelay";
import type { ProspectChanges } from "./businessRecordMutations";
import type { TenantCommunityId } from "@/features/communities/tenantCommunity";

export type LeadContext = {
  channelId: string;
  communityId: TenantCommunityId;
  currentPubkey: string | null;
  records: BusinessRecords;
  refresh: () => Promise<void>;
  role: BusinessRole | null;
};

export type ProspectRow = BusinessRecords["prospects"][number];

export const STAGES: Array<{ value: ProspectStage; label: string }> = [
  { value: "qualified", label: "Qualified" },
  { value: "in_conversation", label: "In conversation" },
  { value: "proposal", label: "Proposal" },
  { value: "won", label: "Won" },
  { value: "lost", label: "Lost" },
];

export type LeadPillTone = "neutral" | "green" | "blue" | "red" | "amber";

export function canEdit(role: BusinessRole | null): boolean {
  return role === "owner" || role === "admin" || role === "member";
}

export function labelForStage(
  stage: ProspectStage,
  qualification: string,
  unreviewedLabel = "Candidate",
): string {
  if (qualification === "unreviewed") return unreviewedLabel;
  return STAGES.find((item) => item.value === stage)?.label ?? stage;
}

/** Pill colour for a lead's stage, as the Leads board draws them. */
export function toneForStage(
  stage: ProspectStage,
  qualification: string,
): LeadPillTone {
  if (qualification === "unreviewed") return "neutral";
  switch (stage) {
    case "won":
      return "green";
    case "lost":
      return "red";
    case "proposal":
      return "amber";
    default:
      return "blue";
  }
}

/** The letter in a business's mark tile. */
export function leadInitial(name: string): string {
  return name.trim().slice(0, 1).toLocaleUpperCase();
}

/** `https://form-field.example/` reads as `form-field.example`. */
export function displayWebsite(website: string): string {
  return website.replace(/^https?:\/\//i, "").replace(/\/$/, "");
}

/**
 * Evidence on a reserved `.example` host is illustrative; anything else is a
 * real source the lead was found at.
 */
export function isIllustrativeEvidence(url: string): boolean {
  try {
    const host = new URL(url).hostname.toLocaleLowerCase();
    return host === "example" || host.endsWith(".example");
  } catch {
    return false;
  }
}

export function createChanges(
  name: string,
  industry: string,
  location: string,
): ProspectChanges {
  const prospectId = crypto.randomUUID();
  return {
    prospectId,
    displayName: name,
    partyType: "organization",
    industry,
    vertical: industry,
    fitScore: null,
    origin: "manual",
    potentialMonthlyValueMinor: null,
    website: null,
    contactName: null,
    location: location || null,
    email: null,
    phone: null,
    evidence: [],
    lastVerifiedAt: null,
    qualification: "unreviewed",
    saved: false,
    stage: "qualified",
    lostReason: null,
  };
}
