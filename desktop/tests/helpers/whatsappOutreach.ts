import type { Page } from "@playwright/test";
import { bytesToHex } from "@noble/hashes/utils.js";
import { generateSecretKey, getPublicKey } from "nostr-tools/pure";
import realTemplateSeed from "../fixtures/whatsapp/wa-templates.json" with {
  type: "json",
};

import { installMockBridge } from "./bridge";
import {
  REFERENCE_AGENTS,
  REFERENCE_CHANNEL_IDS,
  REFERENCE_PROSPECT_IDS,
} from "../../src/testing/e2eReferenceWorkspace";
import { canManageWhatsAppContact } from "../../src/features/whatsapp-outreach/whatsappContactAccess";
import {
  cedarCafeLeadRecord,
  planeTreeCustomerRecord,
} from "../../src/testing/cofounderCustomerFixtures";

declare global {
  interface Window {
    __BUZZ_E2E_WHATSAPP_STATE__?: {
      commands: Array<{ kind: number; body: Record<string, unknown> }>;
      connectCalls: number;
      holdConnect?: boolean;
      releaseConnect?: () => void;
    };
  }
}

export const OUTREACH_CHANNEL_ID = "e2e-whatsapp-outreach-channel";
export const WHATSAPP_BATCH_ID = "e2e-whatsapp-batch";
export const WHATSAPP_SECOND_BATCH_ID = "e2e-whatsapp-batch-second";
export const WHATSAPP_DECIDED_BATCH_ID = "0b2fdeca-0000-4000-8000-000000000018";
export const PLANE_TREE_WA_ID = "27821237135";
export const PLANE_TREE_LEAD_ID = REFERENCE_PROSPECT_IDS["form-field"];
const PLANE_TREE_REPLY_WINDOW_CLOSES_AT = new Date(
  Date.now() + 21 * 60 * 60 * 1000,
).toISOString();
export const CORNER_COTTAGE_WA_ID = "27725551188";
export const JACARANDA_WA_ID = "27821234410";
export const JACARANDA_LEAD_ID = REFERENCE_PROSPECT_IDS.stillroom;
export const LAVENDER_HILL_WA_ID = "27795552241";
export const GREEN_SHUTTER_WA_ID = "27605553422";
export const OAKLEAF_WA_ID = "27655551175";
const E2E_SELF_PUBKEY = "deadbeef".repeat(8);

function deskCustomerRecordMock() {
  const companyRelaySecret = generateSecretKey();
  const planeTreeRecord = planeTreeCustomerRecord({
    scoutPubkey: REFERENCE_AGENTS.scout.pubkey,
    ayaPubkey: REFERENCE_AGENTS.aya.pubkey,
    ownerPubkey: E2E_SELF_PUBKEY,
  });
  planeTreeRecord.partyId = PLANE_TREE_LEAD_ID;
  planeTreeRecord.leadId = PLANE_TREE_LEAD_ID;
  const cornerCottageRecord = cedarCafeLeadRecord({
    scoutPubkey: REFERENCE_AGENTS.scout.pubkey,
    ayaPubkey: REFERENCE_AGENTS.aya.pubkey,
    ownerPubkey: E2E_SELF_PUBKEY,
  });
  cornerCottageRecord.partyId = REFERENCE_PROSPECT_IDS["sunday-edit"];
  cornerCottageRecord.leadId = REFERENCE_PROSPECT_IDS["sunday-edit"];
  cornerCottageRecord.displayName = "Corner Cottage Parkhurst";
  return {
    relaySelf: getPublicKey(companyRelaySecret),
    companyMemberRelayPrivateKeyHex: bytesToHex(companyRelaySecret),
    companyCustomerRecords: [planeTreeRecord, cornerCottageRecord],
    cofounderSettings: {
      enabled: true,
      moneyPreference: "cheapest_first" as const,
      approvalSwitches: {
        approveEveryHire: false,
        approvePlaybookChanges: false,
        approveEveryDiscoveryCampaign: false,
      },
    },
  };
}

export type WhatsAppProofTheme = "light" | "dark";
type WhatsAppMockActor = {
  businessName?: string;
  connected?: boolean;
  relayRole?: "owner" | "admin" | "member" | null;
  profileIsAgent?: boolean;
  profileOwnerPubkey?: string | null;
  workspaceName?: string;
  /** Seed the approved desk states without changing legacy inbox fixtures. */
  deskFixture?: boolean;
  /** A plain relay member can see an open Outreach channel without membership. */
  outreachChannelMember?: boolean;
  batchVariant?:
    | "decided-card-pending"
    | "decided-card-approved"
    | "decided-card-rejected"
    | "decided-card-sent"
    | "decided-card-missing";
};

export async function setWhatsAppProofTheme(
  page: Page,
  theme: WhatsAppProofTheme,
  businessName = "Horizon Labs",
) {
  const relayUrl = (
    process.env.BUZZ_E2E_RELAY_URL ?? "http://localhost:3000"
  ).replace(/^http/u, "ws");
  await page.emulateMedia({ colorScheme: theme });
  await page.addInitScript(
    ({ selectedTheme, scopedRelayUrl, selectedBusinessName }) => {
      const themeName = selectedTheme === "dark" ? "buzz-dark" : "buzz";
      const pubkey = "deadbeef".repeat(8);
      // Appearance snapshots are newer durable preferences than the theme key.
      // Clear prior-run snapshots and the synchronous CSS cache so each proof
      // theme is applied from the scoped preference below.
      for (const key of Object.keys(window.localStorage)) {
        if (key.startsWith("colony.appearance.v1:")) {
          window.localStorage.removeItem(key);
        }
      }
      window.localStorage.removeItem("buzz-theme-cache");
      window.localStorage.setItem("buzz-theme", themeName);
      window.localStorage.setItem("buzz-follow-system", "false");
      window.localStorage.setItem("buzz-accent-color", "#5B45BE");
      window.localStorage.setItem(
        `buzz-community-theme.v1:${pubkey}:${encodeURIComponent(scopedRelayUrl)}`,
        JSON.stringify({
          version: 1,
          theme: themeName,
          accent: "#5B45BE",
          followSystem: false,
        }),
      );

      const communities = JSON.parse(
        window.localStorage.getItem("buzz-communities") ?? "[]",
      ) as Array<{
        id?: string;
        name?: string;
      }>;
      const activeId = window.localStorage.getItem("buzz-active-community-id");
      const active =
        communities.find((community) => community.id === activeId) ??
        communities[0];
      if (active) {
        active.name = selectedBusinessName;
        window.localStorage.setItem(
          "buzz-communities",
          JSON.stringify(communities),
        );
      }
    },
    {
      selectedTheme: theme,
      scopedRelayUrl: relayUrl,
      selectedBusinessName: businessName,
    },
  );
}

const templateTitles: Record<string, string> = {
  hl_vb: "Built a site for you",
  hl_hs_v1: "Has a website: open to a chat?",
  hl_hs_v2: "Has a website: refresh on your radar?",
  hl_hs_v3: "Has a website: short and friendly",
  hl_ns_v1: "No website: full offer",
  hl_ns_v2: "No website: worth a quick chat?",
  hl_ns_v3: "No website: I run Horizon Labs",
  hl_ns_v4: "No website: like to explore?",
  hl_ns_v5: "No website: get set up online",
  hl_fu_v1: "Follow-up: no pressure",
  hl_fu_v2: "Follow-up: checking in",
  hl_pay: "Deposit payment link",
};
const templates = realTemplateSeed
  .filter((template) => template.name.startsWith("hl_"))
  .map((template) => {
    const slots = [...template.body.matchAll(/\{\{(\d+)\}\}/gu)].reduce(
      (highest, match) => Math.max(highest, Number(match[1])),
      0,
    );
    return [
      template.name,
      templateTitles[template.name] ?? template.name.replaceAll("_", " "),
      template.category,
      slots,
      template.body,
      template.buttons,
    ] as const;
  });

const visualBatchLeads = [
  [
    "Jacaranda Lane Guesthouse",
    "+27 82 123 4410",
    "stillroom",
    "jacaranda-lane",
  ],
  ["Koppie View Lodge", "+27 71 555 2286", "north-note", "koppie-view"],
  [
    "The Seventh Street Rooms",
    "+27 83 555 9031",
    "studio-local",
    "seventh-street",
  ],
  [
    "Bramble & Brick Guesthouse",
    "+27 72 555 5517",
    "woven-living",
    "bramble-brick",
  ],
  ["Leafy Stoep Guesthouse", "+27 84 555 1173", "little-kin", "leafy-stoep"],
  ["The Quiet Owl B&B", "+27 76 555 3842", null, "quiet-owl"],
  ["Marigold House", "+27 77 555 6604", "marigold-house", "marigold-house"],
  [
    "Melville Hill Cottages",
    "+27 84 555 7013",
    "melville-hill-cottages",
    "melville-hill-cottages",
  ],
  [
    "Corner Cottage Parkhurst",
    "+27 72 555 1188",
    "sunday-edit",
    "corner-cottage",
  ],
  [
    "Lavender Hill Guesthouse",
    "+27 79 555 2241",
    "gather-house",
    "lavender-hill",
  ],
  [
    "The Green Shutter B&B",
    "+27 60 555 3422",
    "common-ground",
    "green-shutter",
  ],
  ["Oakleaf Rooms", "+27 65 555 1175", "clay-collective", "oakleaf"],
  ["44 Stanley Stay", "+27 67 555 9453", null, "44-stanley"],
  ["Milpark House", "+27 61 555 1166", null, "milpark-house"],
  ["Richmond Avenue B&B", "+27 62 555 0391", null, "richmond-avenue"],
  [
    "Emmarentia Gardens Guesthouse",
    "+27 76 555 1084",
    null,
    "emmarentia-gardens",
  ],
  ["Brixton View Rooms", "+27 73 555 4827", null, "brixton-view"],
  ["Crosby House Guest Lodge", "+27 78 555 6204", null, "crosby-house"],
] as const;

const hlVbBody = String(
  templates.find(([name]) => name === "hl_vb")?.[4] ?? "",
);
const visualBatchContacts = visualBatchLeads.map(
  ([displayName, e164, leadSlug]) => ({
    waId: e164.replace(/\D/gu, ""),
    e164,
    displayName,
    leadId: leadSlug
      ? REFERENCE_PROSPECT_IDS[leadSlug as keyof typeof REFERENCE_PROSPECT_IDS]
      : null,
    ownerPubkey: null,
    threadRootEventId: null,
    lastInboundAt: null,
    windowOpen: false,
    windowClosesAt: null,
    optedOut: false,
    paused: false,
    pauseReason: null,
    unanswered: false,
  }),
);
const visualBatchRows = visualBatchLeads.map(
  ([_displayName, to, leadSlug, slug], idx) => {
    const materialLink = `https://preview.horizonlabs.example/${slug}/`;
    const params = ["guesthouses", "Melville", materialLink];
    const preview = hlVbBody
      .replace("{{1}}", params[0])
      .replace("{{2}}", params[1])
      .replace("{{3}}", params[2]);
    return {
      idx,
      to,
      leadId: leadSlug
        ? REFERENCE_PROSPECT_IDS[
            leadSlug as keyof typeof REFERENCE_PROSPECT_IDS
          ]
        : null,
      params,
      preview,
      state: "ready",
      skipReason: null,
    };
  },
);
const visualBatch = {
  contacts: visualBatchContacts,
  rows: visualBatchRows,
  materialLinks: visualBatchLeads.map(
    ([, , , slug]) => `https://preview.horizonlabs.example/${slug}/`,
  ),
};

const fixture = {
  status: {
    connected: true,
    connectionId: "e2e-whatsapp-connection",
    displayPhoneNumber: "+27 61 534 0627",
    verifiedName: "Horizon Labs",
    quality: "GREEN",
    phoneStatus: "CONNECTED",
    dailyNewChatCap: 30,
    newChatsSentToday: 12,
    sendingPaused: false,
    pauseReason: null,
    issues: [],
    outreachChannelId: OUTREACH_CHANNEL_ID,
    templatesSyncedAt: "2026-10-29T08:00:00+02:00",
    webhookUrl: "https://relay.example/hooks/whatsapp/e2e",
    templates: templates.map(
      ([name, title, category, slots, body, labels]) => ({
        title,
        name,
        proposedTitle: null,
        titleCustom: true,
        language: "en",
        category,
        status: "APPROVED",
        body,
        slots,
        buttons: labels.map((text) => ({ type: "QUICK_REPLY", text })),
      }),
    ),
  },
  contacts: [
    {
      waId: PLANE_TREE_WA_ID,
      e164: "+27 82 123 7135",
      displayName: "Thandeka",
      profileName: "Thandeka",
      businessName: "Plane Tree Guesthouse",
      website: "https://form-field.example",
      leadId: PLANE_TREE_LEAD_ID,
      ownerPubkey: null,
      threadRootEventId: "e2e-whatsapp-thread-plane-tree",
      lastInboundAt: "2026-10-29T09:14:00+02:00",
      windowOpen: true,
      windowClosesAt: PLANE_TREE_REPLY_WINDOW_CLOSES_AT,
      optedOut: false,
      paused: false,
      pauseReason: null,
      unanswered: false,
    },
    {
      waId: CORNER_COTTAGE_WA_ID,
      e164: "+27 72 555 1188",
      displayName: "Lwazi",
      profileName: "Lwazi",
      businessName: "Corner Cottage Parkhurst",
      website: "https://sunday-edit.example",
      leadId: REFERENCE_PROSPECT_IDS["sunday-edit"],
      ownerPubkey: null,
      threadRootEventId: "e2e-whatsapp-thread-corner-cottage",
      lastInboundAt: "2026-10-29T11:48:00+02:00",
      windowOpen: true,
      windowClosesAt: "2026-10-30T11:48:00+02:00",
      optedOut: false,
      paused: false,
      pauseReason: null,
      unanswered: true,
    },
    {
      waId: JACARANDA_WA_ID,
      e164: "+27 82 123 4410",
      displayName: "Mpho",
      profileName: "Mpho",
      businessName: "Jacaranda Lane Guesthouse",
      website: "https://stillroom.example",
      leadId: REFERENCE_PROSPECT_IDS.stillroom,
      ownerPubkey: null,
      threadRootEventId: "e2e-whatsapp-thread-jacaranda",
      lastInboundAt: null,
      windowOpen: false,
      windowClosesAt: null,
      optedOut: false,
      paused: false,
      pauseReason: null,
      unanswered: false,
      awaitingApproval: true,
    },
    {
      waId: LAVENDER_HILL_WA_ID,
      e164: "+27 79 555 2241",
      displayName: "Karen",
      profileName: "Karen",
      businessName: "Lavender Hill Guesthouse",
      leadId: REFERENCE_PROSPECT_IDS["gather-house"],
      ownerPubkey: null,
      threadRootEventId: "e2e-whatsapp-thread-lavender-hill",
      lastInboundAt: null,
      windowOpen: false,
      windowClosesAt: null,
      optedOut: false,
      paused: false,
      pauseReason: null,
      unanswered: false,
    },
    {
      waId: GREEN_SHUTTER_WA_ID,
      e164: "+27 60 555 3422",
      displayName: "Palesa",
      profileName: "Palesa",
      businessName: "The Green Shutter B&B",
      website: "https://common-ground.example",
      leadId: REFERENCE_PROSPECT_IDS["common-ground"],
      ownerPubkey: null,
      threadRootEventId: "e2e-whatsapp-thread-green-shutter",
      lastInboundAt: null,
      windowOpen: false,
      windowClosesAt: null,
      optedOut: false,
      paused: false,
      pauseReason: null,
      unanswered: false,
    },
    {
      waId: OAKLEAF_WA_ID,
      e164: "+27 65 555 1175",
      displayName: "Sam",
      profileName: "Sam",
      businessName: "Oakleaf Rooms",
      website: "https://clay-collective.example",
      leadId: REFERENCE_PROSPECT_IDS["clay-collective"],
      ownerPubkey: null,
      threadRootEventId: "e2e-whatsapp-thread-oakleaf",
      lastInboundAt: null,
      windowOpen: false,
      windowClosesAt: null,
      optedOut: true,
      paused: false,
      pauseReason: null,
      unanswered: false,
    },
    {
      waId: "27825550104",
      e164: "+27 82 555 0104",
      displayName: "Nomsa",
      profileName: "Nomsa",
      leadId: null,
      ownerPubkey: null,
      threadRootEventId: "e2e-whatsapp-thread-parkview",
      lastInboundAt: "2026-10-28T16:20:00+02:00",
      windowOpen: false,
      windowClosesAt: null,
      optedOut: true,
      paused: false,
      pauseReason: null,
      unanswered: false,
    },
    {
      waId: "27615550126",
      e164: "+27 61 555 0126",
      displayName: "+27 61 555 0126",
      profileName: null,
      leadId: null,
      ownerPubkey: null,
      threadRootEventId: "e2e-whatsapp-thread-melville-garden",
      lastInboundAt: "2026-10-28T14:03:00+02:00",
      windowOpen: false,
      windowClosesAt: null,
      optedOut: true,
      paused: false,
      pauseReason: null,
      unanswered: false,
    },
  ],
  batch: {
    batchId: WHATSAPP_BATCH_ID,
    status: "pending",
    createdBy: REFERENCE_AGENTS.scout.pubkey,
    template: "hl_vb",
    language: "en",
    note: "Melville guesthouses are ready for a first hello.",
    materialLinks: ["https://preview.horizonlabs.example/jacaranda-lane/"],
    decidedBy: null,
    decidedAt: null,
    createdAt: "2026-10-29T10:46:00+02:00",
    nextSendAt: null,
    rows: [
      {
        idx: 0,
        to: "+27 82 123 4410",
        leadId: JACARANDA_LEAD_ID,
        params: [
          "guesthouses",
          "Melville, Johannesburg",
          "https://preview.horizonlabs.example/jacaranda-lane/",
        ],
        preview:
          "Hey, my name's Pam from Horizon Labs 👋 I came across your business while looking for guesthouses in Melville, Johannesburg and went ahead and built something for you — take a look: https://preview.horizonlabs.example/jacaranda-lane/\n\nThis site could be yours. We can tailor it to your business and get it live quickly. Interested?",
        state: "ready",
        skipReason: null,
      },
      {
        idx: 1,
        to: "+27 71 555 2286",
        leadId: null,
        params: ["guesthouses", "Melville, Johannesburg"],
        preview:
          "Hey, I'm Pam from Horizon Labs 👋 I found your business while looking for guesthouses in Melville, Johannesburg and had a look at your website. We help businesses like yours improve their online presence — better website, stronger Google rankings, Google Business Profile, and AI tools to make day-to-day easier. Open to a quick chat?",
        state: "ready",
        skipReason: null,
      },
      {
        idx: 2,
        to: "+27 83 555 9031",
        leadId: null,
        params: ["guesthouses", "Melville, Johannesburg"],
        preview:
          "Hey, I'm Pam from Horizon Labs 👋 I found your business while looking for guesthouses in Melville, Johannesburg and had a look at your website. We help businesses like yours improve their online presence — better website, stronger Google rankings, Google Business Profile, and AI tools to make day-to-day easier. Open to a quick chat?",
        state: "ready",
        skipReason: null,
      },
    ],
    counts: { total: 3, ready: 3, sent: 0, failed: 0, dropped: 0 },
  },
  threads: {
    [PLANE_TREE_WA_ID]: {
      contact: null,
      messages: [
        {
          id: "e2e-wa-agent-template",
          waId: PLANE_TREE_WA_ID,
          direction: "out",
          kind: "template",
          text: "Hey, my name's Pam from Horizon Labs. I was looking for guesthouses in Parkhurst and came across your business. We specialise in improving websites, boosting Google rankings, getting Google Business Profiles sorted, and helping local businesses use AI to run things more efficiently. Is that something you'd be open to a quick chat about?",
          templateName: "hl_hs_v1",
          state: "read",
          errorCode: null,
          errorText: null,
          eventId: "e2e-wa-agent-template-event",
          senderPubkey: REFERENCE_AGENTS.scout.pubkey,
          createdAt: "2026-10-28T09:40:00+02:00",
        },
        {
          id: "e2e-wa-inbound-button",
          waId: PLANE_TREE_WA_ID,
          direction: "in",
          kind: "button",
          text: "Yes, tell me more",
          templateName: null,
          state: "read",
          errorCode: null,
          errorText: null,
          eventId: "e2e-wa-inbound-button-event",
          senderPubkey: null,
          createdAt: "2026-10-29T08:52:00+02:00",
        },
        {
          id: "e2e-wa-failed",
          waId: PLANE_TREE_WA_ID,
          direction: "out",
          kind: "media",
          text: "plane-tree-preview.png · Image · 6.2 MB",
          templateName: null,
          state: "failed",
          errorCode: "131026",
          errorText: "Not delivered. WhatsApp only takes images up to 5 MB.",
          eventId: "e2e-wa-failed-event",
          senderPubkey: REFERENCE_AGENTS.aya.pubkey,
          createdAt: "2026-10-29T08:55:00+02:00",
        },
        {
          id: "e2e-wa-agent-sent",
          waId: PLANE_TREE_WA_ID,
          direction: "out",
          kind: "text",
          text: "Great! Here’s how your site could look. It works well on phones and the Book now button comes straight to you, no booking fees.",
          templateName: null,
          state: "delivered",
          errorCode: null,
          errorText: null,
          eventId: "e2e-wa-agent-event",
          senderPubkey: REFERENCE_AGENTS.aya.pubkey,
          createdAt: "2026-10-29T08:56:00+02:00",
        },
        {
          id: "e2e-wa-inbound",
          waId: PLANE_TREE_WA_ID,
          direction: "in",
          kind: "text",
          text: "This looks great! What would it cost?",
          templateName: null,
          state: "read",
          errorCode: null,
          errorText: null,
          eventId: "e2e-wa-inbound-event",
          senderPubkey: null,
          createdAt: "2026-10-29T09:14:00+02:00",
        },
        {
          id: "e2e-wa-owner-reply",
          waId: PLANE_TREE_WA_ID,
          direction: "out",
          kind: "text",
          text: "Glad you like it, Thandeka. I’ll send you the options this afternoon. Is a quick call at 14:00 okay?",
          templateName: null,
          state: "sent",
          errorCode: null,
          errorText: null,
          eventId: "e2e-wa-owner-reply-event",
          senderPubkey: E2E_SELF_PUBKEY,
          createdAt: "2026-10-29T12:10:00+02:00",
        },
        {
          id: "e2e-wa-agent-draft",
          waId: PLANE_TREE_WA_ID,
          direction: "out",
          kind: "text",
          text: "Hi Thandeka, here’s what the new site includes: pages that work well on phones, a Book now button that comes straight to you, and your Google Business Profile set up. Basheer will call you at 14:00 to go through the options.",
          templateName: null,
          state: "draft",
          errorCode: null,
          errorText: null,
          eventId: null,
          senderPubkey: REFERENCE_AGENTS.aya.pubkey,
          createdAt: "2026-10-29T12:14:00+02:00",
        },
        {
          id: "e2e-wa-agent-draft-reject",
          waId: PLANE_TREE_WA_ID,
          direction: "out",
          kind: "text",
          text: "Would Tuesday morning work for a quick call?",
          templateName: null,
          state: "draft",
          errorCode: null,
          errorText: null,
          eventId: null,
          senderPubkey: REFERENCE_AGENTS.aya.pubkey,
          createdAt: "2026-10-29T12:16:00+02:00",
        },
      ],
    },
  },
  commands: [] as Array<{ kind: number; body: Record<string, unknown> }>,
  connectCalls: 0,
  holdConnect: false,
  releaseConnect: undefined as (() => void) | undefined,
};

const secondPendingBatch = {
  ...fixture.batch,
  batchId: WHATSAPP_SECOND_BATCH_ID,
  createdAt: "2026-10-29T09:46:00+02:00",
  rows: fixture.batch.rows.slice(1, 2).map((row, idx) => ({ ...row, idx })),
  counts: { total: 1, ready: 1, sent: 0, failed: 0, dropped: 0 },
};

const deskFirstBatchRows = [
  visualBatchLeads[0],
  visualBatchLeads[9],
  visualBatchLeads[10],
  visualBatchLeads[1],
  visualBatchLeads[2],
  visualBatchLeads[3],
  visualBatchLeads[4],
  visualBatchLeads[5],
  visualBatchLeads[6],
  visualBatchLeads[7],
  visualBatchLeads[8],
  visualBatchLeads[11],
].map(([name, to, leadSlug, slug], idx) => {
  const materialLink = `https://preview.horizonlabs.example/${slug}/`;
  const leadId = leadSlug
    ? REFERENCE_PROSPECT_IDS[leadSlug as keyof typeof REFERENCE_PROSPECT_IDS]
    : null;
  const preview =
    idx === 0
      ? "Hi, I'm Basheer from Horizon Labs. I came across Jacaranda Lane Guesthouse on Google Maps and built you a website, to show you what's possible. If you'd like to see it, is it okay if I give you a call?"
      : `Hi, I'm Basheer from Horizon Labs. I found ${name} while looking for guesthouses in Melville and built a website to show you what's possible. Would you be open to a quick call?`;
  return {
    idx,
    to,
    leadId,
    params: ["guesthouses", "Melville", materialLink],
    preview,
    state: "ready",
    skipReason: null,
  };
});

const deskFollowUpRows = [
  {
    idx: 0,
    to: "+27 65 555 1175",
    leadId: REFERENCE_PROSPECT_IDS["clay-collective"],
    params: [],
    preview:
      "Hi Sam, just checking in after my note about Oakleaf Rooms. No pressure if now is not the right time.",
    state: "ready",
    skipReason: null,
  },
  {
    idx: 1,
    to: "+27 82 555 0104",
    leadId: null,
    params: [],
    preview:
      "Hi Nomsa, just checking in after my note about Parkview Guest Rooms. No pressure if now is not the right time.",
    state: "ready",
    skipReason: null,
  },
  {
    idx: 2,
    to: "+27 76 333 4444",
    leadId: null,
    params: [],
    preview:
      "Hi Kabelo, just checking in after my note about Melville Garden Rooms. No pressure if now is not the right time.",
    state: "ready",
    skipReason: null,
  },
];

const deskPipelineAdditionalContacts = [
  {
    waId: "27794441101",
    e164: "+27 79 444 1101",
    displayName: "Kea",
    profileName: "Kea",
    businessName: "Linden Tree Lodge",
    website: "https://linden-tree-lodge.example",
    label: null,
    leadId: REFERENCE_PROSPECT_IDS["linden-tree-lodge"],
    ownerPubkey: null,
    threadRootEventId: "e2e-whatsapp-thread-linden-loft",
    lastInboundAt: null,
    lastContactedAt: "2026-10-28T09:30:00+02:00",
    lastOutboundState: "read",
    lastReplyAt: null,
    stage: "contacted",
    stageSetBy: "auto",
    stageChangedAt: "2026-10-28T09:30:00+02:00",
    windowOpen: false,
    windowClosesAt: null,
    optedOut: false,
    paused: false,
    pauseReason: null,
    unanswered: false,
    awaitingApproval: false,
  },
  {
    waId: "27664441102",
    e164: "+27 66 444 1102",
    displayName: "Zola",
    profileName: "Zola",
    businessName: "Greenside Courtyard Stay",
    website: "https://greenside-courtyard.example",
    label: null,
    leadId: REFERENCE_PROSPECT_IDS["greenside-courtyard"],
    ownerPubkey: null,
    threadRootEventId: "e2e-whatsapp-thread-greenside-cottage",
    lastInboundAt: null,
    lastContactedAt: "2026-10-28T09:20:00+02:00",
    lastOutboundState: null,
    lastReplyAt: null,
    stage: "contacted",
    stageSetBy: "auto",
    stageChangedAt: "2026-10-28T09:20:00+02:00",
    windowOpen: false,
    windowClosesAt: null,
    optedOut: false,
    paused: false,
    pauseReason: null,
    unanswered: false,
    awaitingApproval: false,
  },
  {
    waId: "27734441103",
    e164: "+27 73 444 1103",
    displayName: "Sipho",
    profileName: "Sipho",
    businessName: "Emmarentia Gardens Guesthouse",
    website: "https://emmarentia-gardens.example",
    label: null,
    leadId: REFERENCE_PROSPECT_IDS["emmarentia-gardens"],
    ownerPubkey: null,
    threadRootEventId: "e2e-whatsapp-thread-emmarentia-rose",
    lastInboundAt: "2026-10-29T10:18:00+02:00",
    lastContactedAt: "2026-10-27T09:30:00+02:00",
    lastOutboundState: null,
    lastReplyAt: "2026-10-29T10:18:00+02:00",
    stage: "replied",
    stageSetBy: "auto",
    stageChangedAt: "2026-10-29T10:18:00+02:00",
    windowOpen: true,
    windowClosesAt: "2026-10-30T10:18:00+02:00",
    optedOut: false,
    paused: false,
    pauseReason: null,
    unanswered: false,
    awaitingApproval: false,
  },
  {
    waId: "27724441104",
    e164: "+27 72 444 1104",
    displayName: "Pieter",
    profileName: "Pieter",
    businessName: "Westcliff Garden Suites",
    website: "https://westcliff-garden-suites.example",
    label: null,
    leadId: REFERENCE_PROSPECT_IDS["westcliff-garden-suites"],
    ownerPubkey: null,
    threadRootEventId: "e2e-whatsapp-thread-westcliff",
    lastInboundAt: "2026-10-30T14:00:00+02:00",
    lastContactedAt: "2026-10-27T09:00:00+02:00",
    lastOutboundState: null,
    lastReplyAt: "2026-10-30T14:00:00+02:00",
    stage: "call_booked",
    stageSetBy: "auto",
    stageChangedAt: "2026-10-30T14:00:00+02:00",
    windowOpen: true,
    windowClosesAt: "2026-10-31T14:00:00+02:00",
    optedOut: false,
    paused: false,
    pauseReason: null,
    unanswered: false,
    awaitingApproval: false,
  },
  {
    waId: "27763334444",
    e164: "+27 76 333 4444",
    displayName: "Kabelo",
    profileName: "Kabelo",
    businessName: "Melville Hill Cottages",
    website: "https://melville-hill-cottages.example",
    label: null,
    leadId: REFERENCE_PROSPECT_IDS["melville-hill-cottages"],
    ownerPubkey: null,
    threadRootEventId: "e2e-whatsapp-thread-melville-garden-business",
    lastInboundAt: null,
    lastContactedAt: "2026-10-22T10:00:00+02:00",
    lastOutboundState: null,
    lastReplyAt: null,
    stage: "quote_sent",
    stageSetBy: "auto",
    stageChangedAt: "2026-10-22T10:00:00+02:00",
    windowOpen: false,
    windowClosesAt: null,
    optedOut: false,
    paused: false,
    pauseReason: null,
    unanswered: false,
    awaitingApproval: false,
  },
  {
    waId: "27714441105",
    e164: "+27 71 444 1105",
    displayName: "Ayesha",
    profileName: "Ayesha",
    businessName: "The Stoop on Seventh",
    website: "https://stoop-on-seventh.example",
    label: null,
    leadId: REFERENCE_PROSPECT_IDS["stoop-on-seventh"],
    ownerPubkey: null,
    threadRootEventId: "e2e-whatsapp-thread-craighall",
    lastInboundAt: null,
    lastContactedAt: "2026-10-15T10:00:00+02:00",
    lastOutboundState: null,
    lastReplyAt: "2026-10-16T11:00:00+02:00",
    stage: "client",
    stageSetBy: "auto",
    stageChangedAt: "2026-10-16T11:00:00+02:00",
    windowOpen: false,
    windowClosesAt: null,
    optedOut: false,
    paused: false,
    pauseReason: null,
    unanswered: false,
    awaitingApproval: false,
  },
  {
    waId: "27684441106",
    e164: "+27 68 444 1106",
    displayName: "Palesa",
    profileName: "Palesa",
    businessName: "The Quiet Owl B&B",
    website: "https://quiet-owl-bb.example",
    label: null,
    leadId: REFERENCE_PROSPECT_IDS["quiet-owl-bb"],
    ownerPubkey: null,
    threadRootEventId: "e2e-whatsapp-thread-quiet-owl",
    lastInboundAt: "2026-10-27T14:00:00+02:00",
    lastContactedAt: "2026-10-26T09:00:00+02:00",
    lastOutboundState: null,
    lastReplyAt: "2026-10-27T14:00:00+02:00",
    stage: "no_thanks",
    stageSetBy: "auto",
    stageChangedAt: "2026-10-27T14:00:00+02:00",
    windowOpen: false,
    windowClosesAt: null,
    optedOut: true,
    paused: false,
    pauseReason: null,
    unanswered: false,
    awaitingApproval: false,
  },
  {
    waId: "27874441107",
    e164: "+27 87 444 1107",
    displayName: "Naledi",
    profileName: "Naledi",
    businessName: "Parkview Manor",
    website: "https://parkview-manor.example",
    label: null,
    leadId: REFERENCE_PROSPECT_IDS["parkview-manor"],
    ownerPubkey: null,
    threadRootEventId: "e2e-whatsapp-thread-parkview-manor",
    lastInboundAt: "2026-10-26T11:00:00+02:00",
    lastContactedAt: "2026-10-25T09:00:00+02:00",
    lastOutboundState: null,
    lastReplyAt: "2026-10-26T11:00:00+02:00",
    stage: "no_thanks",
    stageSetBy: "auto",
    stageChangedAt: "2026-10-26T11:00:00+02:00",
    windowOpen: false,
    windowClosesAt: null,
    optedOut: true,
    paused: false,
    pauseReason: null,
    unanswered: false,
    awaitingApproval: false,
  },
  {
    waId: "27884441108",
    e164: "+27 88 444 1108",
    displayName: "Musa",
    profileName: "Musa",
    businessName: "Bramble & Brick Guesthouse",
    website: "https://woven-living.example",
    label: null,
    leadId: REFERENCE_PROSPECT_IDS["woven-living"],
    ownerPubkey: null,
    threadRootEventId: "e2e-whatsapp-thread-westdene",
    lastInboundAt: "2026-10-24T11:00:00+02:00",
    lastContactedAt: "2026-10-24T09:00:00+02:00",
    lastOutboundState: null,
    lastReplyAt: "2026-10-24T11:00:00+02:00",
    stage: "stopped",
    stageSetBy: "auto",
    stageChangedAt: "2026-10-24T11:00:00+02:00",
    windowOpen: false,
    windowClosesAt: null,
    optedOut: true,
    paused: false,
    pauseReason: null,
    unanswered: false,
    awaitingApproval: false,
  },
  {
    waId: "27894441109",
    e164: "+27 89 444 1109",
    displayName: "Musa",
    profileName: "Musa",
    businessName: "The Red Fern Guesthouse",
    website: "https://red-fern-guesthouse.example",
    label: null,
    leadId: REFERENCE_PROSPECT_IDS["red-fern-guesthouse"],
    ownerPubkey: null,
    threadRootEventId: "e2e-whatsapp-thread-red-fern",
    lastInboundAt: null,
    lastContactedAt: "2026-10-23T09:00:00+02:00",
    lastOutboundState: null,
    lastReplyAt: null,
    stage: "not_on_whatsapp",
    stageSetBy: "auto",
    stageChangedAt: "2026-10-23T09:00:00+02:00",
    windowOpen: false,
    windowClosesAt: null,
    optedOut: false,
    paused: false,
    pauseReason: null,
    unanswered: false,
    awaitingApproval: false,
  },
  {
    waId: "27683735905",
    e164: "+27 68 373 5905",
    displayName: "+27 68 373 5905",
    profileName: null,
    businessName: null,
    website: null,
    label: null,
    leadId: null,
    ownerPubkey: null,
    threadRootEventId: "e2e-whatsapp-thread-unlinked",
    lastInboundAt: "2026-10-29T08:44:00+02:00",
    lastContactedAt: null,
    lastOutboundState: null,
    lastReplyAt: "2026-10-29T08:44:00+02:00",
    stage: "none",
    stageSetBy: "auto",
    stageChangedAt: "2026-10-29T08:44:00+02:00",
    windowOpen: true,
    windowClosesAt: "2026-10-30T08:44:00+02:00",
    optedOut: false,
    paused: false,
    pauseReason: null,
    unanswered: false,
    awaitingApproval: false,
  },
];

export async function installWhatsAppMockBridge(
  page: Page,
  theme: WhatsAppProofTheme = "light",
  actor: WhatsAppMockActor = {},
) {
  const canManageContacts = canManageWhatsAppContact({
    role: actor.relayRole ?? "owner",
    isOutreachMember: actor.outreachChannelMember !== false,
    isManagedAgent: actor.profileIsAgent ?? false,
    ownerPubkey: actor.profileOwnerPubkey ?? null,
  });
  await page.emulateMedia({ colorScheme: theme });
  await installMockBridge(page, {
    referenceWorkspace: true,
    referenceWorkspaceBusinessName:
      actor.businessName ?? actor.workspaceName ?? "Horizon Labs",
    colony3Sidebar: true,
    whatsappGuesthouses: true,
    referenceWorkspaceRole: "owner",
    relayRequiresMembership: true,
    relayRole: actor.relayRole ?? "owner",
    ...(actor.deskFixture ? deskCustomerRecordMock() : {}),
    colony3Extras: {
      profiles: [
        {
          pubkey: E2E_SELF_PUBKEY,
          displayName: "Basheer Phiri",
          isAgent: actor.profileIsAgent ?? false,
          ownerPubkey: actor.profileOwnerPubkey ?? null,
        },
      ],
      channels: [
        {
          id: "e2e-whatsapp-general-channel",
          name: "general",
          description: "The company-wide conversation.",
          visibility: "open",
        },
        {
          id: OUTREACH_CHANNEL_ID,
          name: "Outreach",
          description: `Private WhatsApp conversations for ${actor.businessName ?? "your business"}.`,
          visibility:
            actor.outreachChannelMember === false ? "open" : "private",
          selfIsMember: actor.outreachChannelMember !== false,
          ...(actor.outreachChannelMember === false
            ? {}
            : { selfRole: "owner" }),
        },
        {
          id: "e2e-whatsapp-aya-dm",
          name: "Aya",
          description: "Direct messages with Aya.",
          channelType: "dm",
          members: [{ pubkey: REFERENCE_AGENTS.aya.pubkey, role: "member" }],
          selfRole: "owner",
        },
        {
          id: REFERENCE_CHANNEL_IDS.scoutDm,
          name: "Scout",
          description: "Direct messages with Scout.",
          channelType: "dm",
          members: [{ pubkey: REFERENCE_AGENTS.scout.pubkey, role: "member" }],
          selfRole: "owner",
        },
      ],
    },
  });
  await setWhatsAppProofTheme(
    page,
    theme,
    actor.businessName ?? actor.workspaceName,
  );
  await page.addInitScript(
    (seed) => {
      const clone = <T>(value: T): T => JSON.parse(JSON.stringify(value)) as T;
      const {
        selfPubkey,
        canManageContacts,
        activeLeadIds,
        referenceProspectIds,
        visualBatch: visual,
        visualLeadBatchIds,
        visualNorthNoteLeadId,
        stillroomLeadId,
        scoutPubkey,
        decidedBatchId,
        batchVariant: seededBatchVariant,
        deskFixture,
        deskFirstBatchRows,
        deskFollowUpRows,
        deskPipelineAdditionalContacts,
        templates: templateSeed,
        initiallyConnected,
        ...baseSeed
      } = seed;
      const state = clone(baseSeed);
      const routeSearch = new URLSearchParams(
        window.location.hash.split("?")[1] ?? "",
      );
      const batchVariant =
        seededBatchVariant ??
        new URLSearchParams(window.location.search).get("__waBatch") ??
        routeSearch.get("batch");
      const disconnected =
        initiallyConnected === false ||
        routeSearch.get("whatsapp") === "disconnected";
      const approvedBatch = batchVariant === "approved";
      const sendNowBatch = batchVariant === "send-now";
      const leadsBatch = batchVariant === "leads";
      const heldBatch = batchVariant === "held";
      const identityFallback = batchVariant === "identity";
      const missingJacarandaIdentity =
        heldBatch || identityFallback || sendNowBatch;
      const visualBatchScreen = approvedBatch || batchVariant === "visual";
      state.status.connected = !disconnected;
      state.status.templates = disconnected ? [] : clone(seed.status.templates);
      state.batch.status =
        approvedBatch || sendNowBatch || leadsBatch ? "approved" : "pending";
      state.secondPendingBatch.status = state.batch.status;
      state.batch.decidedBy =
        approvedBatch || sendNowBatch || leadsBatch
          ? "deadbeef".repeat(8)
          : null;
      state.batch.decidedAt =
        approvedBatch || sendNowBatch || leadsBatch
          ? "2026-10-29T10:52:00+02:00"
          : null;
      if (
        batchVariant?.startsWith("decided-card-") &&
        [
          "decided-card-pending",
          "decided-card-approved",
          "decided-card-rejected",
          "decided-card-sent",
          "decided-card-missing",
        ].includes(batchVariant)
      ) {
        const tomorrow = new Intl.DateTimeFormat("en-CA", {
          timeZone: "Africa/Johannesburg",
          year: "numeric",
          month: "2-digit",
          day: "2-digit",
        })
          .formatToParts(new Date(Date.now() + 24 * 60 * 60 * 1000))
          .reduce<Record<string, string>>((parts, part) => {
            if (part.type !== "literal") parts[part.type] = part.value;
            return parts;
          }, {});
        const sendsAt = new Date(
          Date.UTC(
            Number(tomorrow.year),
            Number(tomorrow.month) - 1,
            Number(tomorrow.day),
            6,
          ),
        ).toISOString();
        const sourceRow = state.batch.rows[0];
        state.batch.batchId = decidedBatchId;
        state.batch.rows = [
          {
            ...sourceRow,
            idx: 0,
            state: batchVariant === "decided-card-sent" ? "sent" : "ready",
            sendsAt,
          },
        ];
        state.batch.counts = {
          total: 1,
          ready: batchVariant === "decided-card-sent" ? 0 : 1,
          sent: batchVariant === "decided-card-sent" ? 1 : 0,
          failed: 0,
          dropped: 0,
        };
        state.batch.nextSendAt = sendsAt;
        if (batchVariant === "decided-card-approved") {
          state.batch.status = "approved";
          state.batch.decidedBy = selfPubkey;
          state.batch.decidedAt = "2026-10-29T10:52:00+02:00";
        } else if (batchVariant === "decided-card-rejected") {
          state.batch.status = "rejected";
          state.batch.decidedBy = selfPubkey;
          state.batch.decidedAt = "2026-10-29T10:52:00+02:00";
        } else if (batchVariant === "decided-card-sent") {
          state.batch.status = "completed";
          state.batch.decidedBy = selfPubkey;
          state.batch.decidedAt = "2026-10-29T10:52:00+02:00";
        } else if (batchVariant === "decided-card-missing") {
          state.batch.batchId = "missing-decided-card-batch";
        }
        state.secondPendingBatch.batchId = "different-e2e-pending-batch";
        state.secondPendingBatch.status = "pending";
      }
      const baseContacts = clone(seed.contacts);
      const deskContacts = deskFixture
        ? clone(deskPipelineAdditionalContacts)
        : [];
      if (visualBatchScreen) {
        state.status.newChatsSentToday = 16;
        state.contacts = clone(visual.contacts);
        state.batch.rows = clone(visual.rows);
        state.batch.materialLinks = clone(visual.materialLinks);
        state.batch.counts = {
          total: 18,
          ready: 18,
          sent: 0,
          failed: 0,
          dropped: 0,
        };
      }
      if (leadsBatch) {
        const batchLeadIds = new Set(visualLeadBatchIds);
        state.contacts = [
          ...baseContacts,
          {
            waId: "27715552286",
            e164: "+27 71 555 2286",
            displayName: "Koppie View Lodge",
            leadId: visualNorthNoteLeadId,
            ownerPubkey: null,
            threadRootEventId: "e2e-whatsapp-thread-koppie-view",
            lastInboundAt: null,
            windowOpen: false,
            windowClosesAt: null,
            optedOut: false,
            paused: false,
            pauseReason: null,
            unanswered: false,
          },
        ];
        state.batch.rows = visual.rows
          .filter((row) => row.leadId && batchLeadIds.has(row.leadId))
          .map((row, index) => ({
            ...row,
            state: index === 0 ? "ready" : "held",
            skipReason: index === 0 ? null : "The daily cap has been reached.",
          }));
        state.batch.counts = {
          total: state.batch.rows.length,
          ready: 1,
          sent: 0,
          failed: 0,
          dropped: 0,
        };
      }
      if (approvedBatch) {
        state.status.newChatsSentToday = 22;
        state.batch.nextSendAt = "2026-10-29T11:40:00+02:00";
        state.batch.rows = state.batch.rows.map((row, index) => ({
          ...row,
          state:
            index < 2
              ? "read"
              : index < 5
                ? "delivered"
                : index === 5
                  ? "sent"
                  : index === 7 || (index >= 14 && index < 17)
                    ? "held"
                    : "ready",
          skipReason:
            index === 7 || (index >= 14 && index < 17)
              ? "The daily cap has been reached."
              : null,
        }));
        state.batch.counts = {
          total: 18,
          ready: 8,
          sent: 6,
          failed: 0,
          dropped: 0,
        };
      }
      if (sendNowBatch) {
        const sendsAt = new Date(Date.now() + 60 * 60 * 1000).toISOString();
        state.status.newChatsSentToday = 4;
        state.batch.nextSendAt = sendsAt;
        state.batch.rows = state.batch.rows.map((row) => ({
          ...row,
          state: "ready",
          sendsAt,
        }));
        state.batch.counts = {
          total: state.batch.rows.length,
          ready: state.batch.rows.length,
          sent: 0,
          failed: 0,
          dropped: 0,
        };
      }
      const planeTreeWaId = baseContacts[0].waId;
      if (deskFixture) {
        state.status.newChatsSentToday = 16;
        state.status.templates = state.status.templates.map((template) => {
          if (template.name === "hl_vb") {
            return {
              ...template,
              buttons: [
                { type: "QUICK_REPLY", text: "Yes, call me" },
                { type: "QUICK_REPLY", text: "No thanks" },
                { type: "QUICK_REPLY", text: "Stop messages" },
              ],
            };
          }
          if (template.name === "hl_fu_v1") {
            return { ...template, title: "Quick follow up" };
          }
          return template;
        });
        state.batch.note = "Message 12 guesthouses in Melville";
        state.batch.rows = clone(deskFirstBatchRows).map((row) =>
          row.to.replace(/\D/gu, "") === "27795552241"
            ? { ...row, leadId: null }
            : row,
        );
        state.batch.materialLinks = deskFirstBatchRows
          .map((row) => row.params[2])
          .filter((link): link is string => Boolean(link));
        state.batch.counts = {
          total: 12,
          ready: 12,
          sent: 0,
          failed: 0,
          dropped: 0,
        };
        state.secondPendingBatch = {
          ...state.secondPendingBatch,
          status: "pending",
          template: "hl_fu_v1",
          note: "Follow up with 3 guesthouses that read but did not reply",
          materialLinks: [],
          rows: clone(deskFollowUpRows),
          counts: {
            total: 3,
            ready: 3,
            sent: 0,
            failed: 0,
            dropped: 0,
          },
        };
        const planeTree = baseContacts.find(
          (contact) => contact.waId === planeTreeWaId,
        );
        if (planeTree) {
          Object.assign(planeTree, {
            stage: "replied",
            stageSetBy: "auto",
            stageChangedAt: "2026-10-29T09:14:00+02:00",
            lastContactedAt: "2026-10-28T09:40:00+02:00",
            lastReplyAt: "2026-10-29T09:14:00+02:00",
            unanswered: true,
          });
        }
        const cornerCottage = baseContacts.find(
          (contact) => contact.waId === "27725551188",
        );
        if (cornerCottage) {
          Object.assign(cornerCottage, {
            displayName: "Lindiwe",
            profileName: "Lindiwe",
            businessName: "Corner Cottage Parkhurst",
            website: "https://sunday-edit.example",
            leadId: referenceProspectIds["sunday-edit"],
            optedOut: false,
            stage: "wants_call",
            stageSetBy: "auto",
            stageChangedAt: "2026-10-29T08:45:00+02:00",
            lastContactedAt: "2026-10-28T09:40:00+02:00",
            lastReplyAt: "2026-10-29T08:45:00+02:00",
            lastInboundAt: "2026-10-29T08:45:00+02:00",
            unanswered: true,
          });
        }
        const jacaranda = baseContacts.find(
          (contact) => contact.waId === "27821234410",
        );
        if (jacaranda) {
          Object.assign(jacaranda, {
            displayName: "Mpho",
            stage: "contacted",
            stageSetBy: "auto",
            stageChangedAt: "2026-10-29T09:40:00+02:00",
            lastContactedAt: "2026-10-29T09:40:00+02:00",
            lastOutboundState: "read",
          });
        }
        const greenShutter = baseContacts.find(
          (contact) => contact.waId === "27605553422",
        );
        if (greenShutter) {
          Object.assign(greenShutter, {
            displayName: "Palesa",
            optedOut: false,
            stage: "contacted",
            stageSetBy: "auto",
            stageChangedAt: "2026-10-28T10:30:00+02:00",
            lastContactedAt: "2026-10-28T10:30:00+02:00",
            lastOutboundState: null,
            unanswered: false,
          });
        }
        const oakleaf = baseContacts.find(
          (contact) => contact.waId === "27655551175",
        );
        if (oakleaf) {
          Object.assign(oakleaf, {
            displayName: "Sam",
            optedOut: false,
            stage: "contacted",
            stageSetBy: "auto",
            stageChangedAt: "2026-10-28T10:30:00+02:00",
            lastContactedAt: "2026-10-28T10:30:00+02:00",
          });
        }
        const parkview = baseContacts.find(
          (contact) => contact.waId === "27825550104",
        );
        if (parkview) {
          Object.assign(parkview, {
            optedOut: false,
            displayName: "Nomsa",
            profileName: "Nomsa",
            businessName: "Parkview Guest Rooms",
            website: "https://parkview-guest-rooms.example",
            leadId: referenceProspectIds["parkview-guest-rooms"],
            stage: "quote_sent",
            stageSetBy: "auto",
            stageChangedAt: "2026-10-23T10:00:00+02:00",
            lastContactedAt: "2026-10-23T10:00:00+02:00",
          });
        }
        const melville = baseContacts.find(
          (contact) => contact.waId === "27615550126",
        );
        if (melville) {
          Object.assign(melville, {
            optedOut: false,
            displayName: "+27 61 555 0126",
            profileName: null,
            businessName: null,
            website: null,
            leadId: null,
            stage: "none",
            stageSetBy: "auto",
            stageChangedAt: null,
            lastContactedAt: null,
          });
        }
        const lavender = baseContacts.find(
          (contact) => contact.waId === "27795552241",
        );
        if (lavender) {
          Object.assign(lavender, {
            leadId: referenceProspectIds["gather-house"],
            businessName: "Lavender Hill Guesthouse",
            profileName: "Karen Mokoena",
            displayName: "Karen Mokoena",
            website: null,
            stage: "contacted",
            stageSetBy: "auto",
            stageChangedAt: "2026-10-28T10:30:00+02:00",
            lastContactedAt: "2026-10-28T10:30:00+02:00",
          });
        }
        state.contacts = clone([
          ...baseContacts.filter((contact) => contact.waId === "27725551188"),
          ...baseContacts.filter((contact) => contact.waId === planeTreeWaId),
          ...baseContacts.filter(
            (contact) =>
              contact.waId !== "27725551188" &&
              contact.waId !== planeTreeWaId &&
              contact.waId !== "27615550126",
          ),
          ...deskContacts,
        ]);
      }
      const templatesForMock = clone(state.status.templates);
      state.threads[planeTreeWaId].contact = clone(seed.contacts[0]);
      for (const contact of baseContacts.slice(1)) {
        state.threads[contact.waId] = { contact: clone(contact), messages: [] };
      }
      for (const contact of deskContacts) {
        state.threads[contact.waId] = { contact: clone(contact), messages: [] };
      }
      if (deskFixture) {
        state.threads[planeTreeWaId].contact = clone(baseContacts[0]);
        const cornerCottage = baseContacts.find(
          (contact) => contact.waId === "27725551188",
        );
        if (cornerCottage && state.threads[cornerCottage.waId]) {
          state.threads[cornerCottage.waId].messages = [
            {
              id: "e2e-wa-corner-cottage-quick-reply",
              waId: cornerCottage.waId,
              direction: "in",
              kind: "button",
              text: "Yes, call me",
              templateName: null,
              state: "read",
              errorCode: null,
              errorText: null,
              eventId: "e2e-wa-corner-cottage-quick-reply-event",
              senderPubkey: null,
              createdAt: "2026-10-29T08:44:00+02:00",
            },
            {
              id: "e2e-wa-corner-cottage-inbound",
              waId: cornerCottage.waId,
              direction: "in",
              kind: "text",
              text: "Can you call me tomorrow morning? Before 10 is best.",
              templateName: null,
              state: "read",
              errorCode: null,
              errorText: null,
              eventId: "e2e-wa-corner-cottage-inbound-event",
              senderPubkey: null,
              createdAt: "2026-10-29T08:45:00+02:00",
            },
          ];
        }
      }
      const jacarandaContact = missingJacarandaIdentity
        ? state.contacts.find((contact) => contact.waId === "27821234410")
        : null;
      const jacarandaThread = missingJacarandaIdentity
        ? state.threads["27821234410"]
        : null;
      if (jacarandaContact && jacarandaThread) {
        jacarandaContact.displayName =
          jacarandaContact.profileName ?? jacarandaContact.e164;
        jacarandaContact.leadId = null;
        jacarandaThread.contact = {
          ...jacarandaThread.contact,
          displayName:
            jacarandaThread.contact.profileName ?? jacarandaThread.contact.e164,
          leadId: null,
        };
        delete jacarandaContact.businessName;
        delete jacarandaContact.website;
        delete jacarandaThread.contact.businessName;
        delete jacarandaThread.contact.website;
        const templateBody =
          templateSeed.find(([name]) => name === "hl_vb")?.[4] ?? "";
        const params = [
          "guesthouses",
          "Melville, Johannesburg",
          "https://preview.horizonlabs.example/jacaranda-lane/",
        ];
        const heldPreview = templateBody.replace(
          /\{\{(\d+)\}\}/gu,
          (_slot, index: string) => params[Number(index) - 1] ?? "",
        );
        const heldBatchRow = state.batch.rows.find(
          (row) => row.to === jacarandaContact.e164,
        );
        if (heldBatchRow) {
          heldBatchRow.preview = heldPreview;
          if (heldBatch) heldBatchRow.state = "held";
          heldBatchRow.leadId = stillroomLeadId;
        }
        jacarandaThread.messages = [
          {
            id: "e2e-wa-jacaranda-sent-template",
            waId: "27821234410",
            direction: "out",
            kind: "template",
            text: heldPreview,
            templateName: "hl_vb",
            state: "sent",
            errorCode: null,
            errorText: null,
            eventId: "e2e-wa-jacaranda-sent-template-event",
            senderPubkey: scoutPubkey,
            createdAt: "2026-10-29T09:40:00+02:00",
          },
        ];
        if (heldBatch || sendNowBatch) {
          jacarandaThread.messages.push({
            id: "e2e-wa-jacaranda-held-template",
            waId: "27821234410",
            direction: "out",
            kind: "template",
            text: heldPreview,
            templateName: "hl_vb",
            state: "queued",
            errorCode: null,
            errorText: null,
            eventId: "e2e-wa-jacaranda-held-template-event",
            senderPubkey: scoutPubkey,
            createdAt: "2026-10-30T09:40:00+02:00",
            ...(sendNowBatch && heldBatchRow?.sendsAt
              ? { sendsAt: heldBatchRow.sendsAt }
              : {}),
          });
        }
      }
      const activeLeadIdSet = new Set(activeLeadIds as string[]);
      const normalizeContactView = (value: unknown) => {
        if (typeof value !== "object" || value === null) return;
        const contact = value as Record<string, unknown>;
        const label =
          typeof contact.label === "string" && contact.label.trim()
            ? contact.label
            : null;
        const profileName =
          typeof contact.profileName === "string" &&
          /[\p{L}\p{N}]/u.test(contact.profileName)
            ? contact.profileName
            : null;
        contact.profileName = profileName;
        contact.displayName =
          label ??
          profileName ??
          (typeof contact.e164 === "string" ? contact.e164 : null);
        contact.lastOutboundState ??= null;
        if (
          typeof contact.leadId !== "string" ||
          !activeLeadIdSet.has(contact.leadId)
        ) {
          delete contact.businessName;
          delete contact.website;
        }
      };
      for (const contact of state.contacts) normalizeContactView(contact);
      for (const thread of Object.values(state.threads)) {
        normalizeContactView(thread.contact);
      }
      const globalWindow = window as Window & {
        __BUZZ_E2E_WHATSAPP_STATE__?: typeof state;
      };
      globalWindow.__BUZZ_E2E_WHATSAPP_STATE__ = state;
      window.__BUZZ_E2E_WHATSAPP__ = async (request) => {
        if (request.action === "post") {
          if (request.path === "/api/whatsapp/connect") {
            state.connectCalls += 1;
            if (state.holdConnect) {
              await new Promise<void>((resolve) => {
                state.releaseConnect = resolve;
              });
            }
            state.status.connected = true;
            state.status.templates = clone(templatesForMock);
            return clone(state.status);
          }
          if (request.path === "/api/whatsapp/disconnect") {
            state.status.connected = false;
            state.status.templates = [];
            return clone(state.status);
          }
          if (request.path !== "/api/whatsapp/query") return {};
          const body = request.body ?? {};
          switch (body.what) {
            case "status":
              return clone(state.status);
            case "templates":
              return {
                templates: clone(
                  state.status.connected ? templatesForMock : [],
                ),
              };
            case "contacts":
              return { contacts: clone(state.contacts) };
            case "batches": {
              const requestedState = body.state;
              const batches = [state.batch, state.secondPendingBatch]
                .filter(
                  (batch) =>
                    requestedState === undefined ||
                    requestedState === batch.status,
                )
                .map((batch) => ({
                  batchId: batch.batchId,
                  status: batch.status,
                  createdBy: batch.createdBy,
                  template: batch.template,
                  language: batch.language,
                  rows: batch.counts,
                  createdAt: batch.createdAt,
                }));
              return {
                batches,
              };
            }
            case "batch":
              return clone(
                [state.batch, state.secondPendingBatch].find(
                  (batch) => batch.batchId === body.batchId,
                ) ?? state.batch,
              );
            case "drafts":
              return {
                drafts: clone(
                  state.threads[planeTreeWaId].messages
                    .filter((message) => message.state === "draft")
                    .map((message) => ({
                      ...message,
                      to: seed.contacts[0].e164,
                      params: [],
                    })),
                ),
              };
            case "thread":
              return clone(
                state.threads[String(body.waId)] ?? {
                  contact: null,
                  messages: [],
                },
              );
            default:
              return {};
          }
        }

        const kind = request.kind ?? 0;
        const body = request.body ?? {};
        state.commands.push({ kind, body: clone(body) });
        if (kind === 47051) {
          if (typeof body.dailyNewChatCap === "number")
            state.status.dailyNewChatCap = body.dailyNewChatCap;
          if (typeof body.sendingPaused === "boolean")
            state.status.sendingPaused = body.sendingPaused;
        }
        if (kind === 47052 && typeof body.waId === "string") {
          const contact = state.contacts.find(
            (candidate) => candidate.waId === body.waId,
          );
          if (contact) {
            if (!canManageContacts) {
              return {
                message:
                  "only the owner, admins, their agents and Outreach members can change a contact's stage or name",
              };
            }
            if (
              typeof body.label === "string" &&
              body.label.trim().length === 0
            ) {
              return { message: "label is empty; send null to clear it" };
            }
            if (typeof body.label === "string" && body.label.length > 120) {
              return { message: "label must be 120 characters or fewer" };
            }
            const manualStages = [
              "contacted",
              "replied",
              "wants_call",
              "call_booked",
              "quote_sent",
              "client",
              "no_thanks",
            ];
            if (
              typeof body.stage === "string" &&
              ["stopped", "no_thanks", "not_on_whatsapp"].includes(
                contact.stage,
              )
            ) {
              return {
                message:
                  "this business asked not to be messaged or is not on WhatsApp, so its stage cannot be changed",
              };
            }
            if (
              typeof body.stage === "string" &&
              !manualStages.includes(body.stage)
            ) {
              return { message: "invalid WhatsApp contact stage" };
            }
            if (
              body.label !== undefined &&
              body.label !== null &&
              typeof body.label !== "string"
            ) {
              return { message: "invalid WhatsApp contact label" };
            }
            if (typeof body.stage === "string") {
              contact.stage = body.stage;
              contact.stageSetBy = selfPubkey;
              contact.stageChangedAt = new Date().toISOString();
            }
            if (body.label === null || typeof body.label === "string") {
              contact.label = body.label;
              const profileName =
                typeof contact.profileName === "string" &&
                /[\p{L}\p{N}]/u.test(contact.profileName)
                  ? contact.profileName
                  : contact.e164;
              contact.displayName = body.label ?? profileName;
            }
            const thread = state.threads[contact.waId];
            if (thread) thread.contact = clone(contact);
          }
        }
        if (kind === 47049 && typeof body.draftId === "string") {
          const draft = state.threads[planeTreeWaId].messages.find(
            (message) => message.id === body.draftId,
          );
          if (draft) {
            if (typeof body.text === "string") draft.text = body.text;
            draft.state = body.decision === "approve" ? "sent" : "rejected";
          }
        }
        if (kind === 47049 && body.batchId === state.batch.batchId) {
          if (body.decision === "send_now") return {};
          const dropRows = Array.isArray(body.dropRows)
            ? (body.dropRows as number[])
            : [];
          const edits = Array.isArray(body.editRows)
            ? (body.editRows as Array<{ idx: number; params: string[] }>)
            : [];
          const templateBody =
            templateSeed.find(([name]) => name === state.batch.template)?.[4] ??
            "";
          state.batch.rows = state.batch.rows.map((row) => {
            const edit = edits.find((value) => value.idx === row.idx);
            const dropped = dropRows.includes(row.idx);
            const rolled = row.idx === 2;
            const keepReadyAfterApprove =
              batchVariant?.startsWith("decided-card-") &&
              body.decision === "approve";
            const params = edit?.params ?? row.params;
            return {
              ...row,
              params,
              preview: templateBody.replace(
                /\{\{(\d+)\}\}/gu,
                (_slot, index: string) => params[Number(index) - 1] ?? "",
              ),
              state: dropped
                ? "dropped"
                : rolled
                  ? "held"
                  : keepReadyAfterApprove
                    ? "ready"
                    : row.idx === 0
                      ? "sent"
                      : "ready",
              skipReason: rolled ? "The daily cap has been reached." : null,
            };
          });
          state.batch.status =
            body.decision === "approve" ? "approved" : "rejected";
          state.batch.decidedBy = selfPubkey;
          state.batch.decidedAt = "2026-10-29T10:52:00+02:00";
          state.batch.nextSendAt = "2026-10-29T11:40:00+02:00";
          const sent = state.batch.rows.filter(
            (row) => row.state === "sent",
          ).length;
          const dropped = state.batch.rows.filter(
            (row) => row.state === "dropped",
          ).length;
          const ready = state.batch.rows.filter(
            (row) => row.state === "ready",
          ).length;
          state.batch.counts = {
            total: state.batch.rows.length,
            ready,
            sent,
            failed: 0,
            dropped,
          };
        }
        if (
          kind === 47049 &&
          body.batchId === state.secondPendingBatch.batchId &&
          (body.decision === "approve" || body.decision === "reject")
        ) {
          state.secondPendingBatch.status =
            body.decision === "approve" ? "approved" : "rejected";
          state.secondPendingBatch.decidedBy = selfPubkey;
          state.secondPendingBatch.decidedAt = "2026-10-29T10:52:00+02:00";
        }
        return {};
      };
    },
    {
      ...fixture,
      initiallyConnected: actor.connected,
      selfPubkey: E2E_SELF_PUBKEY,
      canManageContacts,
      activeLeadIds: Object.values(REFERENCE_PROSPECT_IDS),
      referenceProspectIds: REFERENCE_PROSPECT_IDS,
      templates,
      visualBatch,
      secondPendingBatch,
      visualLeadBatchIds: [
        REFERENCE_PROSPECT_IDS["marigold-house"],
        REFERENCE_PROSPECT_IDS["melville-hill-cottages"],
      ],
      visualNorthNoteLeadId: REFERENCE_PROSPECT_IDS["north-note"],
      stillroomLeadId: REFERENCE_PROSPECT_IDS.stillroom,
      scoutPubkey: REFERENCE_AGENTS.scout.pubkey,
      decidedBatchId: WHATSAPP_DECIDED_BATCH_ID,
      batchVariant: actor.batchVariant ?? null,
      deskFixture: actor.deskFixture === true,
      deskFirstBatchRows,
      deskFollowUpRows,
      deskPipelineAdditionalContacts,
    },
  );
}

export async function gotoWhatsAppScreen(
  page: Page,
  hashPath: string,
  query = "",
) {
  const route = hashPath.startsWith("/") ? hashPath : `/${hashPath}`;
  const separator = route.includes("?") ? "&" : "?";
  const search = query ? `${separator}${query}` : "";
  const batchVariant = new URLSearchParams(query).get("batch");
  const fixtureSearch = batchVariant
    ? `?__waBatch=${encodeURIComponent(batchVariant)}`
    : "";
  await page.goto(`/${fixtureSearch}#${route}${search}`, {
    waitUntil: "domcontentloaded",
  });
  await page.reload({ waitUntil: "domcontentloaded" });
}
