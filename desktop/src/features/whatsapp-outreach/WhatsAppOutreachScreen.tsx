import * as React from "react";
import { useNavigate } from "@tanstack/react-router";
import { Settings, UserRound } from "lucide-react";

import {
  customerRecordForContact,
  resolveOutreachContactIdentity,
} from "./outreachDeskUtils";
import { useCustomerRecordCapabilities } from "@/features/cofounder/customerRecordAccess";
import { useCustomerRecords } from "@/features/cofounder/customerHooks";
import { useMyRelayMembershipLookupQuery } from "@/features/community-members/hooks";
import { useCommunities } from "@/features/communities/useCommunities";
import { useBusinessRecords } from "@/features/discovery/useBusinessRecords";
import { useProfileQuery, useUsersBatchQuery } from "@/features/profile/hooks";
import { useIdentityQuery } from "@/shared/api/hooks";
import { canManageWhatsAppContact } from "./whatsappContactAccess";
import { WhatsAppContactRecordPanel } from "./WhatsAppContactRecordPanel";
import { Button } from "@/shared/ui/button";
import { cn } from "@/shared/lib/cn";
import { useIsMobile } from "@/shared/hooks/use-mobile";
import {
  KIND_WHATSAPP_BATCH_DECISION,
  KIND_WHATSAPP_SEND_REQUEST,
  KIND_WHATSAPP_SETTINGS,
} from "@/shared/constants/kinds";
import {
  getWhatsAppDailyCapProgress,
  isWhatsAppPhoneLikeName,
  maskWhatsAppNumber,
  type WhatsAppBatchDetail,
  type WhatsAppContact,
  type WhatsAppTemplate,
} from "./whatsappOutreachModels";
import {
  contactStatus,
  pendingBatchRowsForContact,
  uniquePendingBatchRowForContact,
} from "./whatsappOutreachContactState";
import {
  useWhatsAppSendNowAccess,
  useWhatsAppSendNowBatchIds,
} from "./whatsappSendNowHooks";
import { WhatsAppPendingBatches } from "./WhatsAppPendingBatches";
import { Tooltip, TooltipContent, TooltipTrigger } from "@/shared/ui/tooltip";
import {
  whatsappInitials,
  WhatsAppThreadMessage,
} from "./WhatsAppThreadMessage";
import { WhatsAppContactRow } from "./WhatsAppContactRow";
import { WhatsAppOutreachContactRow } from "./WhatsAppOutreachContactRow";
import { WhatsAppTemplatePicker } from "./WhatsAppTemplatePicker";
import {
  useWhatsAppCommand,
  useWhatsAppContactsQuery,
  useWhatsAppDraftsQuery,
  useWhatsAppPendingBatchDetailsQuery,
  useWhatsAppStatusQuery,
  useWhatsAppTemplatesQuery,
  useWhatsAppThreadQuery,
} from "./whatsappOutreachQueries";
import "./whatsappOutreach.css";

type ContactFilter =
  | "all"
  | "replied"
  | "sent"
  | "opted-out"
  | "needs-reply"
  | "wants-call"
  | "in-pipeline";

type WhatsAppOutreachScreenProps = {
  channelId: string;
  initialThreadRootId: string | null;
  initialContactWaId?: string | null;
  mode?: "full" | "dock" | "desk";
  channelIsMember?: boolean;
};

function dayKey(value: string | Date): string {
  return new Intl.DateTimeFormat("en-CA", {
    year: "numeric",
    month: "2-digit",
    day: "2-digit",
    timeZone: "Africa/Johannesburg",
  }).format(new Date(value));
}

function messageDayLabel(value: string): string {
  const today = dayKey(new Date());
  const messageDay = dayKey(value);
  const yesterdayDate = new Date();
  yesterdayDate.setDate(yesterdayDate.getDate() - 1);
  if (messageDay === today) return "Today";
  if (messageDay === dayKey(yesterdayDate)) return "Yesterday";
  return new Intl.DateTimeFormat("en-ZA", {
    day: "numeric",
    month: "long",
    timeZone: "Africa/Johannesburg",
  }).format(new Date(value));
}

function windowTimeRemaining(value: string | null): string {
  if (!value) return "";
  const minutes = Math.max(
    0,
    Math.ceil((Date.parse(value) - Date.now()) / 60_000),
  );
  if (minutes >= 60) return `${Math.ceil(minutes / 60)}h left`;
  return `${Math.max(1, minutes)}m left`;
}

export function WhatsAppOutreachScreen({
  channelId,
  initialThreadRootId,
  initialContactWaId = null,
  mode = "full",
  channelIsMember = false,
}: WhatsAppOutreachScreenProps) {
  const navigate = useNavigate();
  const { activeCommunity } = useCommunities();
  const businessName = activeCommunity?.name.trim() || "This business";
  const isMobile = useIsMobile();
  const statusQuery = useWhatsAppStatusQuery();
  const contactsQuery = useWhatsAppContactsQuery();
  const businessData = useBusinessRecords({ createIfMissing: false });
  const templatesQuery = useWhatsAppTemplatesQuery(
    statusQuery.data?.connected === true,
  );
  const draftsQuery = useWhatsAppDraftsQuery();
  const pendingBatchesQuery = useWhatsAppPendingBatchDetailsQuery(
    statusQuery.data?.connected === true,
  );
  const identityQuery = useIdentityQuery();
  const profileQuery = useProfileQuery();
  const currentPubkey = identityQuery.data?.pubkey;
  const membershipQuery = useMyRelayMembershipLookupQuery();
  const customerRecordsQuery = useCustomerRecords(true);
  const customerCapabilities = useCustomerRecordCapabilities();
  const command = useWhatsAppCommand();
  const [selectedWaId, setSelectedWaId] = React.useState<string | null>(null);
  const [activeFilter, setActiveFilter] = React.useState<ContactFilter>("all");
  const [searchTerm, setSearchTerm] = React.useState("");
  const [showContactList, setShowContactList] = React.useState(false);
  const [recordOpen, setRecordOpen] = React.useState(false);
  const [messageText, setMessageText] = React.useState("");
  const [selectedTemplate, setSelectedTemplate] =
    React.useState<WhatsAppTemplate | null>(null);
  const [templateParams, setTemplateParams] = React.useState<string[]>([]);
  const [templatePickerOpen, setTemplatePickerOpen] = React.useState(false);
  const [threadMenuOpen, setThreadMenuOpen] = React.useState(false);
  const [busy, setBusy] = React.useState(false);
  const [actionError, setActionError] = React.useState("");
  const messageListRef = React.useRef<HTMLDivElement>(null);
  const threadMenuButtonRef = React.useRef<HTMLButtonElement>(null);
  const lastContactRequestRef = React.useRef<string | null>(null);

  const contacts = contactsQuery.data ?? [];
  const identityForContact = (contact: WhatsAppContact) =>
    resolveOutreachContactIdentity(contact, businessData.records);
  const templates = templatesQuery.data ?? statusQuery.data?.templates ?? [];
  const role = membershipQuery.data?.membership?.role;
  const canManage =
    membershipQuery.data?.membershipRequired === false ||
    role === "owner" ||
    role === "admin";
  const canEditContact = canManageWhatsAppContact({
    role,
    isOutreachMember: channelIsMember,
    isManagedAgent: Boolean(profileQuery.data?.ownerPubkey),
    ownerPubkey: profileQuery.data?.ownerPubkey,
  });
  const canSendNow = useWhatsAppSendNowAccess({
    role,
    pubkey: profileQuery.isSuccess ? currentPubkey : undefined,
    ownerPubkey: profileQuery.data?.ownerPubkey,
  });
  const status = statusQuery.data;
  const progress = status
    ? getWhatsAppDailyCapProgress(
        status.newChatsSentToday,
        status.dailyNewChatCap,
      )
    : null;

  React.useEffect(() => {
    if (mode === "desk" && isMobile) setShowContactList(true);
  }, [isMobile, mode]);

  React.useEffect(() => {
    if (
      !initialContactWaId ||
      initialContactWaId === lastContactRequestRef.current
    ) {
      return;
    }
    lastContactRequestRef.current = initialContactWaId;
    setSelectedWaId(initialContactWaId);
  }, [initialContactWaId]);

  React.useEffect(() => {
    if (!contacts.length) return;
    if (initialThreadRootId) {
      const matched = contacts.find(
        (contact) => contact.threadRootEventId === initialThreadRootId,
      );
      if (matched) setSelectedWaId(matched.waId);
    } else if (
      !selectedWaId ||
      !contacts.some((contact) => contact.waId === selectedWaId)
    ) {
      setSelectedWaId(
        contacts.some((contact) => contact.waId === initialContactWaId)
          ? initialContactWaId
          : contacts[0].waId,
      );
    }
  }, [contacts, initialContactWaId, initialThreadRootId, selectedWaId]);

  const selectedContact =
    contacts.find((contact) => contact.waId === selectedWaId) ?? null;
  const threadQuery = useWhatsAppThreadQuery(selectedContact?.waId ?? null);
  const thread = threadQuery.data;
  const sendNowBatchIds = useWhatsAppSendNowBatchIds(
    selectedContact,
    status?.connected === true,
    thread?.messages ?? [],
  );
  const pendingBatches = pendingBatchesQuery.data ?? [];
  const selectedPendingBatchRows = selectedContact
    ? pendingBatchRowsForContact(selectedContact, pendingBatches)
    : [];
  const selectedPendingBatchRow = selectedContact
    ? uniquePendingBatchRowForContact(selectedContact, pendingBatches)
    : null;
  const selectedContactWithThreadIdentity = selectedContact
    ? {
        ...selectedContact,
        displayName:
          selectedContact.displayName ?? thread?.contact.displayName ?? null,
        profileName:
          selectedContact.profileName ?? thread?.contact.profileName ?? null,
        businessName:
          selectedContact.businessName ?? thread?.contact.businessName ?? null,
        website: selectedContact.website ?? thread?.contact.website ?? null,
        label: selectedContact.label ?? thread?.contact.label ?? null,
        stage: selectedContact.stage ?? thread?.contact.stage ?? "none",
        stageSetBy:
          selectedContact.stageSetBy ?? thread?.contact.stageSetBy ?? null,
        stageChangedAt:
          selectedContact.stageChangedAt ??
          thread?.contact.stageChangedAt ??
          null,
        lastContactedAt:
          selectedContact.lastContactedAt ??
          thread?.contact.lastContactedAt ??
          null,
        lastOutboundState:
          selectedContact.lastOutboundState ??
          thread?.contact.lastOutboundState ??
          null,
        lastReplyAt:
          selectedContact.lastReplyAt ?? thread?.contact.lastReplyAt ?? null,
        leadId:
          selectedContact.leadId ??
          thread?.contact.leadId ??
          selectedPendingBatchRow?.row.leadId ??
          null,
      }
    : null;
  const selectedIdentity = selectedContactWithThreadIdentity
    ? identityForContact(selectedContactWithThreadIdentity)
    : null;
  const selectedContactIsAnonymous = Boolean(
    selectedIdentity &&
      (!selectedIdentity.isLinked ||
        isWhatsAppPhoneLikeName(selectedIdentity.businessName ?? "")),
  );
  const pendingDrafts = draftsQuery.data ?? [];
  const filteredContacts = contacts.filter((contact) => {
    const visibleContact =
      contact.waId === selectedContactWithThreadIdentity?.waId
        ? selectedContactWithThreadIdentity
        : contact;
    const waitingForApproval =
      pendingBatchRowsForContact(visibleContact, pendingBatches).length > 0;
    const visibleStatus = contactStatus(visibleContact, waitingForApproval);
    const hasDraft = pendingDrafts.some((draft) => draft.waId === contact.waId);
    const matchesFilter =
      mode === "desk" && activeFilter === "needs-reply"
        ? contact.unanswered || hasDraft
        : mode === "desk" && activeFilter === "wants-call"
          ? contact.stage === "wants_call"
          : activeFilter === "opted-out"
            ? visibleStatus === "Opted out"
            : activeFilter === "replied"
              ? visibleStatus === "Replied"
              : activeFilter === "sent"
                ? visibleStatus === "Sent"
                : true;
    const search = searchTerm.trim().toLocaleLowerCase();
    const matchesSearch =
      !search ||
      `${identityForContact(contact).displayName} ${contact.e164} ${identityForContact(contact).personName ?? ""}`
        .toLocaleLowerCase()
        .includes(search);
    return matchesFilter && matchesSearch;
  });
  const selectedDraft =
    pendingDrafts.find((draft) => draft.waId === selectedContact?.waId) ?? null;
  const hasPendingDraft = Boolean(
    selectedDraft ||
      thread?.messages.some((message) => message.state === "draft"),
  );
  const chronologicalMessages = React.useMemo(
    () =>
      [...(thread?.messages ?? [])].sort(
        (left, right) =>
          Date.parse(left.createdAt) - Date.parse(right.createdAt),
      ),
    [thread?.messages],
  );
  const selectedPendingBatch = selectedPendingBatchRow?.batch ?? null;
  const selectedHeldPreview = selectedPendingBatchRow?.row.preview ?? null;
  const selectedAwaitingApproval = selectedPendingBatchRows.length > 0;
  const agentPubkeys = [
    ...new Set(
      chronologicalMessages
        .map((message) => message.senderPubkey)
        .filter((pubkey): pubkey is string =>
          Boolean(pubkey && pubkey !== currentPubkey),
        ),
    ),
  ];
  const agentProfilesQuery = useUsersBatchQuery(agentPubkeys, {
    enabled: agentPubkeys.length > 0,
  });
  const currentUserName = profileQuery.data?.displayName?.trim() || "You";
  const openerTitle = chronologicalMessages.find(
    (message) => message.templateName,
  )?.templateName;
  const openerLabel = templates.find(
    (template) => template.name === openerTitle,
  )?.title;
  const contactFilters: Array<[ContactFilter, string]> =
    mode === "desk"
      ? [
          ["all", "All"],
          ["needs-reply", "Needs reply"],
          ["wants-call", "Wants a call"],
          ["opted-out", "Opted out"],
        ]
      : [
          ["all", "All"],
          ["replied", "Replied"],
          ["sent", "Sent"],
          ["opted-out", "Opted out"],
        ];
  const selectedRecordIdentity = selectedContactWithThreadIdentity
    ? resolveOutreachContactIdentity(
        selectedContactWithThreadIdentity,
        businessData.records,
      )
    : null;
  const selectedCustomerRecord =
    selectedContactWithThreadIdentity && selectedRecordIdentity
      ? customerRecordForContact(
          selectedContactWithThreadIdentity,
          selectedRecordIdentity,
          customerRecordsQuery.data ?? [],
        )
      : null;

  React.useLayoutEffect(() => {
    if (!selectedWaId && !chronologicalMessages.length && !hasPendingDraft)
      return;
    const list = messageListRef.current;
    if (list) list.scrollTop = list.scrollHeight;
  }, [selectedWaId, chronologicalMessages.length, hasPendingDraft]);

  function selectContact(contact: WhatsAppContact) {
    setSelectedWaId(contact.waId);
    setShowContactList(false);
    setRecordOpen(false);
    setMessageText("");
    setSelectedTemplate(null);
    setActionError("");
  }

  function closeThreadMenuOnEscape(
    event: React.KeyboardEvent<HTMLButtonElement>,
  ) {
    if (event.key !== "Escape") return;
    setThreadMenuOpen(false);
    threadMenuButtonRef.current?.focus();
    event.stopPropagation();
  }

  async function approveDraft(draftId: string, text?: string) {
    setActionError("");
    try {
      await command(KIND_WHATSAPP_BATCH_DECISION, {
        v: 1,
        draftId,
        decision: "approve",
        ...(text ? { text } : {}),
      });
    } catch (error) {
      setActionError(
        error instanceof Error
          ? error.message
          : "Could not approve this draft.",
      );
    }
  }

  async function rejectDraft(draftId: string) {
    setActionError("");
    try {
      await command(KIND_WHATSAPP_BATCH_DECISION, {
        v: 1,
        draftId,
        decision: "reject",
      });
    } catch (error) {
      setActionError(
        error instanceof Error
          ? error.message
          : "Could not discard this draft.",
      );
    }
  }

  async function sendMessage(event: React.FormEvent<HTMLFormElement>) {
    event.preventDefault();
    if (
      !selectedContact ||
      !status ||
      busy ||
      selectedContact.optedOut ||
      selectedContact.paused ||
      (hasPendingDraft && !canManage)
    )
      return;
    const trimmed = messageText.trim();
    if (
      (!selectedContact.windowOpen && !selectedTemplate) ||
      (!trimmed && !selectedTemplate)
    )
      return;
    const requestId = crypto.randomUUID();
    setBusy(true);
    setActionError("");
    try {
      await command(KIND_WHATSAPP_SEND_REQUEST, {
        v: 1,
        requestId,
        to: selectedContact.e164,
        ...(selectedTemplate
          ? {
              template: {
                name: selectedTemplate.name,
                language: selectedTemplate.language,
                params: templateParams,
              },
            }
          : { text: trimmed }),
      });
      setMessageText("");
      setSelectedTemplate(null);
      setTemplateParams([]);
    } catch (error) {
      setActionError(
        error instanceof Error
          ? error.message
          : "WhatsApp could not send this message.",
      );
    } finally {
      setBusy(false);
    }
  }

  async function togglePause() {
    if (!status || busy) return;
    setBusy(true);
    setActionError("");
    try {
      await command(KIND_WHATSAPP_SETTINGS, {
        v: 1,
        sendingPaused: !status.sendingPaused,
      });
    } catch (error) {
      setActionError(
        error instanceof Error
          ? error.message
          : "Could not change sending pause.",
      );
    } finally {
      setBusy(false);
    }
  }

  function openLead() {
    const leadId = selectedContactWithThreadIdentity?.leadId;
    if (!leadId) return;
    void navigate({
      to: "/sales/lead/$prospectId",
      params: { prospectId: leadId },
    });
  }

  function reviewBatch(batch: WhatsAppBatchDetail | null) {
    if (!batch) return;
    void navigate({
      to: "/whatsapp/batches/$batchId",
      params: { batchId: batch.batchId },
    });
  }

  async function approveBatch(batchId: string) {
    if (!canManage || busy) return;
    setBusy(true);
    setActionError("");
    try {
      await command(KIND_WHATSAPP_BATCH_DECISION, {
        v: 1,
        batchId,
        decision: "approve",
        dropRows: [],
        editRows: [],
      });
    } catch (error) {
      setActionError(
        error instanceof Error
          ? error.message
          : "Could not approve this WhatsApp batch.",
      );
    } finally {
      setBusy(false);
    }
  }

  function openSettings() {
    void navigate({ to: "/settings", search: { section: "whatsapp" } });
  }

  function chooseTemplate(template: WhatsAppTemplate) {
    setSelectedTemplate(template);
    setTemplateParams(
      Array.from({ length: template.slots }, (_, index) =>
        index === 0
          ? selectedContact
            ? (identityForContact(selectedContact).personName ?? "")
            : ""
          : "",
      ),
    );
    setTemplatePickerOpen(false);
  }

  if (
    statusQuery.isPending ||
    contactsQuery.isPending ||
    (status?.connected && pendingBatchesQuery.isPending)
  ) {
    return (
      <div aria-busy="true" className="c3wa-loading c3wa-outreach-loading">
        Loading WhatsApp conversations…
      </div>
    );
  }

  if (
    statusQuery.isError ||
    contactsQuery.isError ||
    (status?.connected && pendingBatchesQuery.isError)
  ) {
    const error =
      statusQuery.error ?? contactsQuery.error ?? pendingBatchesQuery.error;
    return (
      <main className="c3wa-root c3wa-outreach-error" role="alert">
        {error?.message ?? "WhatsApp conversations could not be loaded."}
      </main>
    );
  }

  if (!status?.connected) {
    return (
      <main className="c3wa-root c3wa-outreach-empty">
        <h1>WhatsApp is not connected</h1>
        <p>Connect your {businessName} number in Settings to open Outreach.</p>
        <Button onClick={openSettings} type="button">
          Open WhatsApp settings
        </Button>
      </main>
    );
  }

  return (
    <main
      className={cn(
        "c3wa-root c3wa-outreach",
        mode === "desk" && "c3wa-outreach-desk",
      )}
      data-has-selected-contact={selectedContact ? "true" : "false"}
      data-mode={mode}
      data-testid="whatsapp-outreach"
      data-channel-id={channelId}
    >
      <aside
        className={cn(
          "c3wa-contact-panel",
          showContactList && !recordOpen && "is-mobile-visible",
          recordOpen && "is-mobile-hidden",
        )}
        aria-label="WhatsApp contacts"
      >
        <header className="c3wa-contact-panel-head">
          {mode === "desk" ? null : (
            <WhatsAppPendingBatches
              canManage={canManage}
              placement="outreach"
              templates={templates}
            />
          )}
          <div className="c3wa-outreach-title-row">
            <div>
              <h1>{mode === "full" ? "Outreach" : "Chats"}</h1>
              <span>WhatsApp · {status.displayPhoneNumber}</span>
            </div>
            <Tooltip disableHoverableContent>
              <TooltipTrigger asChild>
                <button
                  aria-label="WhatsApp settings"
                  className="c3wa-icon-button c3wa-settings-button"
                  data-testid="whatsapp-settings-trigger"
                  onClick={openSettings}
                  title="WhatsApp settings"
                  type="button"
                >
                  <Settings aria-hidden="true" size={17} strokeWidth={1.8} />
                </button>
              </TooltipTrigger>
              <TooltipContent>WhatsApp settings</TooltipContent>
            </Tooltip>
          </div>
          <input
            aria-label="Search chats"
            className="c3wa-contact-search"
            onChange={(event) => setSearchTerm(event.currentTarget.value)}
            placeholder="Search chats"
            type="search"
            value={searchTerm}
          />
          <div
            aria-label="Filter contacts"
            className="c3wa-contact-filters"
            role="tablist"
          >
            {contactFilters.map(([filter, label]) => (
              <button
                aria-selected={activeFilter === filter}
                key={filter}
                onClick={() => setActiveFilter(filter)}
                role="tab"
                type="button"
              >
                {label}
              </button>
            ))}
          </div>
        </header>
        <div className="c3wa-contact-list">
          {filteredContacts.map((contact) => {
            const isSelected = contact.waId === selectedContact?.waId;
            const listedContact =
              isSelected && selectedContactWithThreadIdentity
                ? selectedContactWithThreadIdentity
                : contact;
            const rowMatches = pendingBatchRowsForContact(
              listedContact,
              pendingBatches,
            );
            const rowBatch = rowMatches.length === 1 ? rowMatches[0] : null;
            const rowContact = {
              ...listedContact,
              leadId: listedContact.leadId ?? rowBatch?.row.leadId ?? null,
            };
            const identity = identityForContact(rowContact);
            const preview =
              rowBatch?.row.preview ??
              (isSelected
                ? (chronologicalMessages.at(-1)?.text ?? null)
                : null);
            const hasDraft = pendingDrafts.some(
              (draft) => draft.waId === contact.waId,
            );
            return mode === "desk" ? (
              <WhatsAppOutreachContactRow
                businessName={identity.displayName}
                canReviewBatch={Boolean(rowBatch)}
                contact={rowContact}
                hasDraft={hasDraft}
                key={contact.waId}
                onReviewBatch={() => reviewBatch(rowBatch?.batch ?? null)}
                onSelect={() => selectContact(contact)}
                personName={identity.personName}
                preview={preview}
                selected={isSelected}
                website={identity.website}
                isDesk
                isLinked={identity.isLinked}
                waitingForApproval={rowMatches.length > 0}
              />
            ) : (
              <WhatsAppContactRow
                businessName={identity.displayName}
                canReviewBatch={Boolean(rowBatch)}
                contact={rowContact}
                hasDraft={hasDraft}
                key={contact.waId}
                onReviewBatch={() => reviewBatch(rowBatch?.batch ?? null)}
                onSelect={() => selectContact(contact)}
                preview={preview}
                selected={isSelected}
                waitingForApproval={rowMatches.length > 0}
              />
            );
          })}
          {filteredContacts.length === 0 ? (
            <p className="c3wa-contact-empty">No contacts match this filter.</p>
          ) : null}
        </div>
      </aside>

      {selectedContact ? (
        <section
          className={cn(
            "c3wa-thread-panel",
            (showContactList || recordOpen) && "is-mobile-hidden",
          )}
          aria-label={`Conversation with ${selectedIdentity?.displayName ?? "WhatsApp contact"}`}
        >
          <div aria-hidden="true" className="c3wa-mobile-safe-area" />
          <header className="c3wa-thread-head">
            <button
              aria-label="Back to WhatsApp contacts"
              className="c3wa-mobile-back"
              onClick={() => setShowContactList(true)}
              type="button"
            >
              ‹
            </button>
            <span aria-hidden="true" className="c3wa-thread-avatar">
              {selectedContactIsAnonymous ? (
                <UserRound aria-hidden="true" size={18} strokeWidth={1.8} />
              ) : (
                whatsappInitials(selectedIdentity?.displayName ?? "WhatsApp")
              )}
            </span>
            <div className="c3wa-thread-head-copy">
              <strong>
                {selectedIdentity?.displayName ?? "WhatsApp contact"}
              </strong>
              <span className="c3wa-thread-head-desktop">
                {selectedContactIsAnonymous
                  ? (selectedIdentity?.location?.split(",")[0].trim() ??
                    "WhatsApp contact")
                  : `${selectedIdentity?.personName ?? "Contact"} · ${maskWhatsAppNumber(selectedContact.e164)}`}
                {!selectedContactIsAnonymous && selectedIdentity?.location
                  ? ` · ${selectedIdentity.location.split(",")[0].trim()}`
                  : ""}
              </span>
              <span className="c3wa-thread-head-mobile">
                WhatsApp ·
                {contactStatus(
                  selectedContactWithThreadIdentity ?? selectedContact,
                  selectedAwaitingApproval,
                )}
              </span>
            </div>
            {mode === "desk" ? (
              <Button
                className="c3wa-record-mobile-open"
                onClick={() => {
                  setRecordOpen(true);
                  setShowContactList(false);
                }}
                size="sm"
                type="button"
                variant="outline"
              >
                Record
              </Button>
            ) : selectedContactWithThreadIdentity?.leadId ? (
              <Button
                className="c3wa-open-lead"
                onClick={openLead}
                size="sm"
                type="button"
                variant="outline"
              >
                Open lead
              </Button>
            ) : null}
            <div className="c3wa-thread-menu">
              <button
                aria-controls="c3wa-thread-menu-panel"
                aria-expanded={threadMenuOpen}
                aria-label="WhatsApp conversation actions"
                onClick={() => setThreadMenuOpen((open) => !open)}
                onKeyDown={closeThreadMenuOnEscape}
                ref={threadMenuButtonRef}
                type="button"
              >
                ⋮
              </button>
              {threadMenuOpen ? (
                <div
                  className="c3wa-thread-menu-content"
                  id="c3wa-thread-menu-panel"
                >
                  <button
                    aria-checked={status.sendingPaused}
                    aria-label="Pause all WhatsApp sending"
                    disabled={!canManage || busy}
                    onClick={() => void togglePause()}
                    onKeyDown={closeThreadMenuOnEscape}
                    role="switch"
                    type="button"
                  >
                    {status.sendingPaused
                      ? "Resume all WhatsApp sending"
                      : "Pause all WhatsApp sending"}
                  </button>
                </div>
              ) : null}
            </div>
          </header>

          {selectedContact.optedOut ? (
            <div className="c3wa-contact-lock" role="status">
              <strong>This contact opted out.</strong>
              <span>No WhatsApp messages can be sent to this number.</span>
            </div>
          ) : selectedContact.paused ? (
            <div className="c3wa-contact-lock" role="status">
              <strong>This contact is paused.</strong>
              <span>
                {selectedContact.pauseReason ??
                  "Sending is paused for this contact."}
              </span>
            </div>
          ) : null}

          {(selectedContactWithThreadIdentity?.leadId ??
          selectedContact.leadId) ? (
            <div className="c3wa-lead-context c3wa-mobile-lead-context">
              <span aria-hidden="true" className="c3wa-context-check">
                ✓
              </span>
              <span>
                <strong>
                  Lead ·{" "}
                  {contactStatus(
                    selectedContactWithThreadIdentity ?? selectedContact,
                    selectedAwaitingApproval,
                  ) === "Replied"
                    ? "In conversation"
                    : contactStatus(
                        selectedContactWithThreadIdentity ?? selectedContact,
                        selectedAwaitingApproval,
                      )}
                </strong>
                <small>
                  {openerLabel
                    ? `Opener: ${openerLabel}`
                    : `${businessName} lead`}
                </small>
              </span>
              <button onClick={openLead} type="button">
                Open lead
              </button>
            </div>
          ) : null}

          <div
            className="c3wa-message-list"
            data-testid="whatsapp-thread-messages"
            ref={messageListRef}
          >
            {threadQuery.isPending ? (
              <p className="c3wa-thread-loading">Loading messages…</p>
            ) : null}
            {threadQuery.isError ? (
              <div className="c3wa-alert" role="alert">
                {threadQuery.error.message}
              </div>
            ) : null}
            {chronologicalMessages.map((message, index) => {
              const previousMessage = chronologicalMessages[index - 1];
              const startsDay =
                !previousMessage ||
                dayKey(previousMessage.createdAt) !== dayKey(message.createdAt);
              return (
                <React.Fragment key={message.id}>
                  {startsDay ? (
                    <div
                      className={cn(
                        "c3wa-thread-day-divider",
                        "is-mobile-history",
                      )}
                    >
                      <span>{messageDayLabel(message.createdAt)}</span>
                    </div>
                  ) : null}
                  <WhatsAppThreadMessage
                    agentName={
                      agentProfilesQuery.data?.profiles[
                        message.senderPubkey?.toLowerCase() ?? ""
                      ]?.displayName?.trim() || "Agent"
                    }
                    contactName={
                      selectedIdentity?.personName ?? "WhatsApp contact"
                    }
                    businessName={businessName}
                    anonymousContact={selectedContactIsAnonymous}
                    currentPubkey={currentPubkey}
                    currentUserName={currentUserName}
                    canSendNow={canSendNow}
                    message={message}
                    mobileHistory={index < chronologicalMessages.length - 3}
                    onApproveDraft={approveDraft}
                    onRejectDraft={rejectDraft}
                    sendNowBatchId={sendNowBatchIds.get(message.id) ?? null}
                    templates={templates}
                    waitingForApproval={
                      message.direction === "out" &&
                      message.state === "queued" &&
                      selectedPendingBatchRows.some(
                        ({ row }) => row.preview === message.text,
                      )
                    }
                  />
                </React.Fragment>
              );
            })}
            {selectedAwaitingApproval ? (
              <article
                aria-label={
                  selectedPendingBatch
                    ? "WhatsApp batch waiting for approval"
                    : "WhatsApp batch unavailable"
                }
                className="c3wa-held-batch-card"
                data-testid="whatsapp-thread-held-batch"
              >
                {selectedPendingBatch ? (
                  <>
                    <div>
                      <strong>Waiting for your approval</strong>
                      <p>{selectedHeldPreview ?? "Loading message preview…"}</p>
                    </div>
                    <div className="c3wa-held-batch-actions">
                      <Button
                        disabled={busy || !canManage}
                        onClick={() =>
                          void approveBatch(selectedPendingBatch.batchId)
                        }
                        size="sm"
                        type="button"
                      >
                        Approve batch
                      </Button>
                      <Button
                        onClick={() => reviewBatch(selectedPendingBatch)}
                        size="sm"
                        type="button"
                        variant="outline"
                      >
                        Review
                      </Button>
                      {!canManage ? (
                        <span>
                          Only the owner or an admin can approve this batch.
                        </span>
                      ) : null}
                    </div>
                  </>
                ) : (
                  <p className="m-0">
                    {selectedPendingBatchRows.length > 1
                      ? "More than one pending batch is available for this chat."
                      : "This batch is no longer available."}
                  </p>
                )}
              </article>
            ) : null}
            {thread &&
            thread.messages.length === 0 &&
            !selectedAwaitingApproval ? (
              <p className="c3wa-thread-empty">No messages yet.</p>
            ) : null}
          </div>

          <div className="c3wa-composer-area">
            {actionError ? (
              <p className="c3wa-action-error" role="alert">
                {actionError}
              </p>
            ) : null}
            {selectedContact.windowOpen ? (
              <div className="c3wa-window-status is-open" role="status">
                <span aria-hidden="true">◷</span>Reply window open
                {selectedContact.windowClosesAt
                  ? `, ${windowTimeRemaining(selectedContact.windowClosesAt)}`
                  : ""}
              </div>
            ) : !selectedContact.optedOut ? (
              <div className="c3wa-window-status is-closed" role="status">
                <span aria-hidden="true">◷</span>Reply window closed. Choose an
                approved template to send a new message.
              </div>
            ) : null}
            {selectedTemplate ? (
              <div className="c3wa-selected-template">
                <div>
                  <span>Template</span>
                  <strong>{selectedTemplate.title}</strong>
                </div>
                <button
                  aria-label="Remove selected template"
                  onClick={() => {
                    setSelectedTemplate(null);
                    setTemplateParams([]);
                  }}
                  type="button"
                >
                  ×
                </button>
                {selectedTemplate.slots ? (
                  <div className="c3wa-template-params">
                    {templateParams
                      .map((param, index) => ({
                        key: `${selectedTemplate.name}:variable:${index + 1}`,
                        index,
                        param,
                      }))
                      .map(({ key, index, param }) => (
                        <label key={key}>
                          <span>Variable {index + 1}</span>
                          <input
                            aria-label={`Template variable ${index + 1}`}
                            onChange={(event) => {
                              const value = event.currentTarget.value;
                              setTemplateParams((current) =>
                                current.map((item, valueIndex) =>
                                  valueIndex === index ? value : item,
                                ),
                              );
                            }}
                            value={param}
                          />
                        </label>
                      ))}
                  </div>
                ) : null}
              </div>
            ) : null}
            {templatePickerOpen ? (
              <WhatsAppTemplatePicker
                onClose={() => setTemplatePickerOpen(false)}
                onSelect={chooseTemplate}
                templates={templates}
              />
            ) : null}
            <form
              className="c3wa-composer"
              onSubmit={(event) => void sendMessage(event)}
            >
              <label className="c3wa-sr-only" htmlFor="wa-reply">
                Reply on WhatsApp to{" "}
                {selectedIdentity?.displayName ?? "contact"}
              </label>
              <textarea
                data-testid="whatsapp-reply-input"
                disabled={
                  !selectedContact.windowOpen ||
                  selectedContact.optedOut ||
                  selectedContact.paused ||
                  (hasPendingDraft && !canManage) ||
                  busy ||
                  Boolean(selectedTemplate)
                }
                id="wa-reply"
                onChange={(event) => setMessageText(event.currentTarget.value)}
                placeholder={
                  selectedContact.optedOut
                    ? "This contact opted out"
                    : selectedContact.windowOpen
                      ? isMobile
                        ? "Reply on WhatsApp…"
                        : `Reply on WhatsApp as ${businessName}`
                      : "Choose a template to send a message"
                }
                rows={2}
                value={messageText}
              />
              <div className="c3wa-composer-toolbar">
                <Button
                  aria-label="Use a template"
                  className="c3wa-mobile-template-action"
                  disabled={
                    selectedContact.optedOut ||
                    selectedContact.paused ||
                    (hasPendingDraft && !canManage) ||
                    busy
                  }
                  onClick={() => setTemplatePickerOpen((open) => !open)}
                  size="sm"
                  type="button"
                  variant="outline"
                >
                  <span
                    aria-hidden="true"
                    className="c3wa-mobile-composer-icon"
                  >
                    +
                  </span>
                  <span className="c3wa-sr-only">Use a template</span>
                </Button>
                <span className="c3wa-composer-spacer" />
                <Button
                  aria-label={
                    busy ? "Sending" : canManage ? "Send" : "Request approval"
                  }
                  className="c3wa-mobile-send-action"
                  disabled={
                    selectedContact.optedOut ||
                    selectedContact.paused ||
                    (hasPendingDraft && !canManage) ||
                    busy ||
                    (!messageText.trim() && !selectedTemplate) ||
                    (!selectedContact.windowOpen && !selectedTemplate)
                  }
                  data-testid="whatsapp-send-message"
                  size="sm"
                  type="submit"
                >
                  <span
                    aria-hidden="true"
                    className="c3wa-mobile-composer-icon"
                  >
                    ↑
                  </span>
                  <span className="c3wa-sr-only">
                    {busy ? "Sending" : canManage ? "Send" : "Request approval"}
                  </span>
                </Button>
              </div>
            </form>
            {selectedContact.windowOpen && progress ? (
              <p className="c3wa-cap-note">
                {progress.sent} of {progress.cap} new chats used today. Replies
                do not count toward the cap.
              </p>
            ) : null}
          </div>
        </section>
      ) : (
        <section
          className={cn(
            "c3wa-thread-panel c3wa-no-contact",
            (showContactList || recordOpen) && "is-mobile-hidden",
          )}
          aria-label="WhatsApp conversation"
        >
          <p>Select a contact to open the conversation.</p>
        </section>
      )}
      {mode === "desk" && selectedContact && selectedRecordIdentity ? (
        <WhatsAppContactRecordPanel
          canEdit={canEditContact}
          canWriteCustomerRecord={customerCapabilities.canWrite}
          contact={selectedContactWithThreadIdentity ?? selectedContact}
          customerRecordsLoaded={customerRecordsQuery.isSuccess}
          identity={selectedRecordIdentity}
          mobileVisible={recordOpen}
          onCloseMobile={() => {
            setRecordOpen(false);
            setShowContactList(false);
          }}
          record={selectedCustomerRecord}
        />
      ) : null}
    </main>
  );
}
