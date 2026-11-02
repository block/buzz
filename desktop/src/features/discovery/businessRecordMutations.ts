import {
  BUSINESS_RECORD_SCHEMA_VERSION,
  businessDTag,
  discoveryTagsField,
  KIND_PROPOSAL_ACCEPTANCE,
  KIND_PROPOSAL_VERSION,
  KIND_PROSPECT_ACTION,
  KIND_SERVICE_ACTION,
  proposalConversionIds,
  proposalVersionDTag,
  type ProspectActivity,
  type ProspectHead,
  type ProspectRecord,
  type ProposalAcceptance,
  type ProposalLine,
  type ProposalVersion,
  type ServiceHead,
  type ServiceRecord,
} from "./businessRecordContract";
import { publishBusinessCommand } from "./businessRecordRelay";
import type { TenantCommunityId } from "@/features/communities/tenantCommunity";

export type ProspectChanges = Omit<ProspectRecord, "party"> & {
  displayName: string;
  partyType: "person" | "organization";
  /** Source links for a new record. An update keeps the record's own ids. */
  externalIds?: string[];
};

export type ProposalTerms = {
  title: string;
  scope: string;
  postsPerMonth: number;
  revisionRounds: number;
  serviceStart: string;
};

export function serializeProposalTerms(terms: ProposalTerms): string {
  return JSON.stringify({ schemaVersion: 1, ...terms });
}

export function parseProposalTerms(value: string): ProposalTerms {
  try {
    const parsed = JSON.parse(value) as Partial<ProposalTerms>;
    if (
      typeof parsed.title === "string" &&
      typeof parsed.scope === "string" &&
      typeof parsed.postsPerMonth === "number" &&
      typeof parsed.revisionRounds === "number" &&
      typeof parsed.serviceStart === "string"
    ) {
      return parsed as ProposalTerms;
    }
  } catch {
    // Older proposal versions use the service description as plain terms.
  }
  return {
    title: "Service proposal",
    scope: value,
    postsPerMonth: 0,
    revisionRounds: 0,
    serviceStart: "",
  };
}

export async function publishServiceAction(input: {
  channelId: string;
  communityId: TenantCommunityId;
  current: { eventId: string; record: ServiceHead } | null;
  serviceId: string;
  name: string;
  description: string;
  monthlyFeeMinor: number;
  postsPerMonth: number;
  revisionRounds: number;
}): Promise<string> {
  const service: ServiceRecord = {
    serviceId: input.serviceId,
    name: input.name.trim(),
    description: input.description,
    currency: "ZAR",
    monthlyFeeMinor: input.monthlyFeeMinor,
    postsPerMonth: input.postsPerMonth,
    revisionRounds: input.revisionRounds,
  };
  return publishBusinessCommand({
    kind: KIND_SERVICE_ACTION,
    channelId: input.channelId,
    dTag: businessDTag(input.communityId, "service", input.serviceId),
    content: {
      schemaVersion: BUSINESS_RECORD_SCHEMA_VERSION,
      serviceId: input.serviceId,
      action: input.current ? "update" : "create",
      expectedHeadEventId: input.current?.eventId ?? null,
      service,
    },
  });
}

export async function publishProspectAction(input: {
  channelId: string;
  communityId: TenantCommunityId;
  current: { eventId: string; record: ProspectHead } | null;
  changes: ProspectChanges;
  activity?: Pick<ProspectActivity, "activityKind" | "content"> &
    Partial<
      Pick<
        ProspectActivity,
        "activityId" | "proposalId" | "proposalVersionEventId"
      >
    >;
}): Promise<string> {
  const prospectId =
    input.current?.record.prospectId ?? input.changes.prospectId;
  const discoveryTags =
    input.changes.discoveryTags ?? input.current?.record.prospect.discoveryTags;
  const prospect: ProspectRecord = {
    prospectId,
    party: {
      partyId: prospectId,
      partyType: input.changes.partyType,
      displayName: input.changes.displayName.trim(),
      externalIds:
        input.current?.record.prospect.party.externalIds ??
        input.changes.externalIds ??
        [],
    },
    industry: input.changes.industry.trim(),
    vertical: input.changes.vertical.trim(),
    fitScore: input.changes.fitScore ?? null,
    origin:
      input.changes.origin ?? input.current?.record.prospect.origin ?? null,
    ...discoveryTagsField(discoveryTags),
    potentialMonthlyValueMinor:
      input.changes.potentialMonthlyValueMinor ?? null,
    website: normalizeOptional(input.changes.website),
    contactName: normalizeOptional(input.changes.contactName),
    location: normalizeOptional(input.changes.location),
    email: normalizeOptional(input.changes.email),
    phone: normalizeOptional(input.changes.phone),
    evidence: input.changes.evidence,
    lastVerifiedAt: input.changes.lastVerifiedAt,
    qualification: input.changes.qualification,
    saved: input.changes.saved,
    stage: input.changes.stage,
    lostReason:
      input.changes.stage === "lost" ? input.changes.lostReason : null,
  };
  return publishBusinessCommand({
    kind: KIND_PROSPECT_ACTION,
    channelId: input.channelId,
    dTag: businessDTag(input.communityId, "prospect", prospectId),
    content: {
      schemaVersion: BUSINESS_RECORD_SCHEMA_VERSION,
      prospectId,
      action: input.current ? "update" : "create",
      expectedHeadEventId: input.current?.eventId ?? null,
      prospect,
      activity: input.activity
        ? {
            activityId: input.activity.activityId ?? crypto.randomUUID(),
            ...input.activity,
            content: input.activity.content.trim(),
            proposalId: input.activity.proposalId ?? null,
            proposalVersionEventId:
              input.activity.proposalVersionEventId ?? null,
          }
        : null,
    },
  });
}

export async function publishProposalVersion(input: {
  channelId: string;
  communityId: TenantCommunityId;
  proposalId: string;
  prospect: ProspectHead;
  current: {
    eventId: string;
    head: { revision: number; currentVersionEventId: string };
  } | null;
  acceptorPubkey: string;
  terms: ProposalTerms;
  monthlyFeeMinor: number;
  serviceId: string | null;
}): Promise<string> {
  const version: ProposalVersion = {
    schemaVersion: BUSINESS_RECORD_SCHEMA_VERSION,
    proposalId: input.proposalId,
    prospectPartyId: input.prospect.prospect.party.partyId,
    namedAcceptorPubkey: input.acceptorPubkey,
    revision: (input.current?.head.revision ?? 0) + 1,
    previousVersionEventId: input.current?.head.currentVersionEventId ?? null,
    expiresAt: null,
    currency: "ZAR",
    lines: [
      {
        serviceId: input.serviceId,
        description: input.terms.title.trim(),
        quantityHundredths: 100,
        unitAmountMinor: input.monthlyFeeMinor,
      } satisfies ProposalLine,
    ],
    terms: serializeProposalTerms(input.terms),
  };
  return publishBusinessCommand({
    kind: KIND_PROPOSAL_VERSION,
    channelId: input.channelId,
    dTag: proposalVersionDTag(
      input.communityId,
      input.proposalId,
      version.revision,
    ),
    content: version,
  });
}

export async function publishProposalAcceptance(input: {
  channelId: string;
  communityId: TenantCommunityId;
  proposal: {
    head: {
      proposalId: string;
      currentVersionEventId: string;
      currentVersionDigest: string;
    };
  };
  acceptedByName: string;
  acceptedAt: number;
  evidenceReference: string;
  exactTermsConfirmed: boolean;
}): Promise<{ eventId: string; acceptance: ProposalAcceptance }> {
  const ids = await proposalConversionIds(
    input.proposal.head.proposalId,
    input.proposal.head.currentVersionEventId,
  );
  const acceptance: ProposalAcceptance = {
    schemaVersion: BUSINESS_RECORD_SCHEMA_VERSION,
    proposalId: input.proposal.head.proposalId,
    proposalVersionEventId: input.proposal.head.currentVersionEventId,
    proposalVersionDigest: input.proposal.head.currentVersionDigest,
    ...ids,
    evidence: {
      acceptedByName: input.acceptedByName.trim(),
      acceptedAt: input.acceptedAt,
      evidenceReference: input.evidenceReference.trim(),
      exactTermsConfirmed: input.exactTermsConfirmed,
    },
  };
  const eventId = await publishBusinessCommand({
    kind: KIND_PROPOSAL_ACCEPTANCE,
    channelId: input.channelId,
    dTag: businessDTag(input.communityId, "conversion", ids.conversionId),
    content: acceptance,
  });
  return { eventId, acceptance };
}

function normalizeOptional(value: string | null | undefined): string | null {
  const normalized = value?.trim() ?? "";
  return normalized ? normalized : null;
}
