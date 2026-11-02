import * as React from "react";
import { ArrowRight, ChevronDown, Clock3, UserRound } from "lucide-react";

import { useBusinessRecords } from "@/features/discovery/useBusinessRecords";
import { BusinessLogoMark } from "@/features/discovery/BusinessLogoMark";
import { customerActions } from "@/features/cofounder/customerActions";
import {
  useCustomerRecordAction,
  useCustomerRecords,
} from "@/features/cofounder/customerHooks";
import { useCustomerRecordCapabilities } from "@/features/cofounder/customerRecordAccess";
import { useCommunities } from "@/features/communities/useCommunities";
import { useMyRelayMembershipLookupQuery } from "@/features/community-members/hooks";
import { useProfileQuery } from "@/features/profile/hooks";
import { Button } from "@/shared/ui/button";
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuTrigger,
} from "@/shared/ui/dropdown-menu";
import { KIND_WHATSAPP_CONTACT_UPDATE } from "@/shared/constants/kinds";
import type { WhatsAppContact, WhatsAppStage } from "./whatsappOutreachModels";
import {
  useWhatsAppCommand,
  useWhatsAppContactsQuery,
  useWhatsAppDraftsQuery,
} from "./whatsappOutreachQueries";
import { canManageWhatsAppContact } from "./whatsappContactAccess";
import {
  isWhatsAppPipelineStage,
  WHATSAPP_MANUAL_STAGES,
  WHATSAPP_PIPELINE_STAGES,
  WHATSAPP_STAGE_LABELS,
} from "./whatsappOutreachStages";
import {
  customerRecordForContact,
  customerRecordNextStep,
  resolveOutreachContactIdentity,
} from "./outreachDeskUtils";

type Props = {
  channelIsMember: boolean;
  onOpenChat: (waId: string) => void;
};

function nextStep(
  contact: WhatsAppContact,
  hasDraft: boolean,
  customerStep: string | null,
  personName: string,
) {
  if (contact.stage === "contacted") {
    return contact.lastOutboundState
      ? `Waiting for a reply, ${contact.lastOutboundState}`
      : null;
  }
  if (contact.stage === "replied") {
    return hasDraft ? "Scout drafted a reply" : "Reply needed";
  }
  if (contact.stage === "wants_call") return `Call ${personName}`;
  return customerStep;
}

function relativeContactDay(value: string | null): string | null {
  if (!value) return null;
  const date = new Date(value);
  if (Number.isNaN(date.getTime())) return null;
  const dayStamp = (input: Date) => {
    const parts = new Intl.DateTimeFormat("en-CA", {
      year: "numeric",
      month: "2-digit",
      day: "2-digit",
      timeZone: "Africa/Johannesburg",
    }).formatToParts(input);
    const fields = Object.fromEntries(
      parts
        .filter((part) => part.type !== "literal")
        .map((part) => [part.type, Number(part.value)]),
    );
    return Date.UTC(fields.year, fields.month - 1, fields.day);
  };
  const days = Math.max(
    0,
    Math.floor((dayStamp(new Date()) - dayStamp(date)) / 86_400_000),
  );
  if (days === 0) return "today";
  if (days === 1) return "yesterday";
  return `${days} days ago`;
}

function PipelineCard({
  contact,
  canEdit,
  hasDraft,
  identity,
  customerStep,
  busy,
  onMove,
  onOpenChat,
}: {
  contact: WhatsAppContact;
  canEdit: boolean;
  hasDraft: boolean;
  identity: ReturnType<typeof resolveOutreachContactIdentity>;
  customerStep: string | null;
  busy: boolean;
  onMove: (contact: WhatsAppContact, stage: WhatsAppStage) => void;
  onOpenChat: (waId: string) => void;
}) {
  const stageLabel = WHATSAPP_STAGE_LABELS[contact.stage];
  const primaryName = identity.displayName;
  const step = nextStep(
    contact,
    hasDraft,
    customerStep,
    identity.personName || primaryName,
  );
  const contactedOn = relativeContactDay(contact.lastContactedAt);
  return (
    <article
      className="c3wa-pipeline-card"
      data-testid={`outreach-pipeline-card-${contact.waId}`}
      draggable={canEdit}
      onDragStart={(event) => {
        event.dataTransfer.effectAllowed = "move";
        event.dataTransfer.setData("text/plain", contact.waId);
      }}
    >
      <div className="c3wa-pipeline-card-head">
        <span aria-hidden="true" className="c3wa-pipeline-logo">
          {identity.isLinked ? (
            <BusinessLogoMark
              className="c3wa-desk-logo-mark"
              name={primaryName}
              website={identity.website}
            />
          ) : (
            <UserRound size={18} />
          )}
        </span>
        <div className="c3wa-pipeline-card-copy">
          <h3>{primaryName}</h3>
          <p>{identity.personName || "No name yet"}</p>
        </div>
        {canEdit ? (
          <DropdownMenu>
            <DropdownMenuTrigger asChild>
              <Button
                aria-label={`Move ${primaryName} to another stage`}
                className="c3wa-pipeline-move-trigger"
                data-testid={`outreach-move-${contact.waId}`}
                disabled={busy}
                size="sm"
                type="button"
                variant="outline"
              >
                <span className="c3wa-sr-only">Move to</span>
                <ChevronDown aria-hidden="true" size={14} />
              </Button>
            </DropdownMenuTrigger>
            <DropdownMenuContent align="end" aria-label="Move to stage">
              {WHATSAPP_MANUAL_STAGES.map((stage) => (
                <DropdownMenuItem
                  key={stage}
                  disabled={stage === contact.stage || busy}
                  onSelect={() => onMove(contact, stage)}
                >
                  {WHATSAPP_STAGE_LABELS[stage]}
                </DropdownMenuItem>
              ))}
            </DropdownMenuContent>
          </DropdownMenu>
        ) : null}
      </div>
      {identity.isLinked ? (
        <p className="c3wa-pipeline-phone">{contact.e164}</p>
      ) : null}
      {contactedOn ? (
        <p className="c3wa-pipeline-last-contacted">
          <Clock3 aria-hidden="true" size={12} />
          Last contacted {contactedOn}
        </p>
      ) : null}
      {!identity.isLinked ? (
        <span className="c3wa-contact-unlinked-chip">
          Not linked to a business
        </span>
      ) : null}
      <div className="c3wa-pipeline-card-foot">
        <span className={`c3wa-pipeline-stage-chip is-${contact.stage}`}>
          {stageLabel}
        </span>
        {step ? (
          contact.stage === "wants_call" ? (
            <a
              className="c3wa-pipeline-next-step"
              href={`tel:+${contact.waId.replace(/\D/gu, "")}`}
            >
              <ArrowRight aria-hidden="true" size={12} />
              {step}
            </a>
          ) : (
            <span className="c3wa-pipeline-next-step">
              <ArrowRight aria-hidden="true" size={12} />
              {step}
            </span>
          )
        ) : null}
      </div>
      <Button
        className="c3wa-pipeline-open-chat"
        onClick={() => onOpenChat(contact.waId)}
        size="sm"
        type="button"
        variant="ghost"
      >
        Open chat
      </Button>
    </article>
  );
}

export function OutreachPipeline({ channelIsMember, onOpenChat }: Props) {
  const { activeCommunity } = useCommunities();
  const businessName = activeCommunity?.name.trim() || "This business";
  const contactsQuery = useWhatsAppContactsQuery();
  const draftsQuery = useWhatsAppDraftsQuery();
  const business = useBusinessRecords({ createIfMissing: false });
  const customerRecords = useCustomerRecords(true);
  const customerAccess = useCustomerRecordCapabilities();
  const customerAction = useCustomerRecordAction();
  const membership = useMyRelayMembershipLookupQuery();
  const profile = useProfileQuery();
  const command = useWhatsAppCommand();
  const [searchTerm, setSearchTerm] = React.useState("");
  const [busyWaId, setBusyWaId] = React.useState<string | null>(null);
  const [actionError, setActionError] = React.useState("");
  const role = membership.data?.membership?.role;
  const canEdit = canManageWhatsAppContact({
    role,
    isOutreachMember: channelIsMember,
    isManagedAgent: Boolean(profile.data?.ownerPubkey),
    ownerPubkey: profile.data?.ownerPubkey,
  });
  const contacts = contactsQuery.data ?? [];
  const drafts = draftsQuery.data ?? [];
  const draftIds = new Set(drafts.map((draft) => draft.waId));
  const search = searchTerm.trim().toLocaleLowerCase();
  const eligible = contacts.filter((contact) => {
    const inPipeline =
      isWhatsAppPipelineStage(contact.stage) ||
      contact.stage === "no_thanks" ||
      contact.stage === "stopped" ||
      contact.stage === "not_on_whatsapp";
    const identity = resolveOutreachContactIdentity(contact, business.records);
    return (
      inPipeline &&
      (!search ||
        `${identity.displayName} ${contact.e164} ${contact.profileName ?? ""}`
          .toLocaleLowerCase()
          .includes(search))
    );
  });
  const pipelineContacts = eligible.filter((contact) =>
    isWhatsAppPipelineStage(contact.stage),
  );
  const collapsedStages = ["no_thanks", "stopped", "not_on_whatsapp"] as const;
  const records = customerRecords.data ?? [];

  async function move(contact: WhatsAppContact, stage: WhatsAppStage) {
    if (!canEdit || busyWaId) return;
    setActionError("");
    setBusyWaId(contact.waId);
    try {
      if (stage === "call_booked") {
        if (customerRecords.isPending || customerRecords.isError) {
          throw new Error(
            "Customer records must load before booking this call.",
          );
        }
        const identity = resolveOutreachContactIdentity(
          contact,
          business.records,
        );
        const record = customerRecordForContact(
          contact,
          identity,
          customerRecords.data ?? [],
        );
        if (record && customerAccess.canWrite) {
          await customerAction.mutateAsync(
            customerActions.recordMilestone(record, {
              milestoneKind: "call_booked",
              at: Math.floor(Date.now() / 1000),
            }),
          );
        }
      }
      await command(KIND_WHATSAPP_CONTACT_UPDATE, {
        v: 1,
        waId: contact.waId,
        stage,
      });
    } catch (cause) {
      setActionError(
        cause instanceof Error ? cause.message : "Could not move this contact.",
      );
    } finally {
      setBusyWaId(null);
    }
  }

  function handleDrop(
    event: React.DragEvent<HTMLElement>,
    stage: WhatsAppStage,
  ) {
    event.preventDefault();
    const waId = event.dataTransfer.getData("text/plain");
    const contact = contacts.find((candidate) => candidate.waId === waId);
    if (contact) void move(contact, stage);
  }

  if (contactsQuery.isError) {
    return (
      <div className="c3wa-root c3wa-desk-error-panel" role="alert">
        Pipeline could not be loaded. {contactsQuery.error.message}
      </div>
    );
  }

  return (
    <main
      className="c3wa-root c3wa-desk c3wa-pipeline"
      data-testid="outreach-desk-pipeline"
    >
      <header className="c3wa-desk-page-head">
        <div>
          <p className="c3wa-desk-eyebrow">{businessName} · WhatsApp</p>
          <h1>Pipeline</h1>
          <p>Move conversations forward as the work happens.</p>
        </div>
        <label className="c3wa-pipeline-search">
          <span className="c3wa-sr-only">Search pipeline</span>
          <input
            onChange={(event) => setSearchTerm(event.currentTarget.value)}
            placeholder="Find a business"
            type="search"
            value={searchTerm}
          />
        </label>
      </header>
      {actionError ? (
        <p className="c3wa-desk-error" role="alert">
          {actionError}
        </p>
      ) : null}
      {contactsQuery.isPending ? (
        <p className="c3wa-desk-empty" role="status">
          Loading pipeline…
        </p>
      ) : null}
      <section
        className="c3wa-pipeline-columns"
        aria-label="WhatsApp pipeline stages"
      >
        {WHATSAPP_PIPELINE_STAGES.map((stage) => {
          const stageContacts = pipelineContacts.filter(
            (contact) => contact.stage === stage,
          );
          return (
            <section
              aria-labelledby={`pipeline-heading-${stage}`}
              className="c3wa-pipeline-column"
              data-stage={stage}
              key={stage}
              onDragOver={(event) => {
                if (canEdit) event.preventDefault();
              }}
              onDrop={(event) => handleDrop(event, stage)}
            >
              <header>
                <h2 id={`pipeline-heading-${stage}`}>
                  {WHATSAPP_STAGE_LABELS[stage]}
                </h2>
                <span>{stageContacts.length}</span>
              </header>
              {stageContacts.length ? (
                stageContacts.map((contact) => {
                  const businessIdentity = resolveOutreachContactIdentity(
                    contact,
                    business.records,
                  );
                  const record = customerRecordForContact(
                    contact,
                    businessIdentity,
                    records,
                  );
                  return (
                    <PipelineCard
                      canEdit={canEdit}
                      contact={contact}
                      customerStep={customerRecordNextStep(record)}
                      hasDraft={draftIds.has(contact.waId)}
                      identity={businessIdentity}
                      key={contact.waId}
                      onMove={(selectedContact, nextStage) =>
                        void move(selectedContact, nextStage)
                      }
                      onOpenChat={onOpenChat}
                      busy={busyWaId === contact.waId}
                    />
                  );
                })
              ) : (
                <p className="c3wa-pipeline-empty-stage">
                  No contacts here yet
                </p>
              )}
            </section>
          );
        })}
      </section>
      <div
        className="c3wa-pipeline-mobile-list"
        data-testid="outreach-pipeline-mobile-list"
      >
        {WHATSAPP_PIPELINE_STAGES.map((stage) => {
          const stageContacts = pipelineContacts.filter(
            (contact) => contact.stage === stage,
          );
          return (
            <details
              className="c3wa-pipeline-mobile-stage"
              key={stage}
              open={stageContacts.length > 0}
            >
              <summary>
                <span>{WHATSAPP_STAGE_LABELS[stage]}</span>
                <b>{stageContacts.length}</b>
              </summary>
              {stageContacts.map((contact) => {
                const businessIdentity = resolveOutreachContactIdentity(
                  contact,
                  business.records,
                );
                const record = customerRecordForContact(
                  contact,
                  businessIdentity,
                  records,
                );
                return (
                  <PipelineCard
                    canEdit={canEdit}
                    contact={contact}
                    customerStep={customerRecordNextStep(record)}
                    hasDraft={draftIds.has(contact.waId)}
                    identity={businessIdentity}
                    key={contact.waId}
                    onMove={(selectedContact, nextStage) =>
                      void move(selectedContact, nextStage)
                    }
                    onOpenChat={onOpenChat}
                    busy={busyWaId === contact.waId}
                  />
                );
              })}
            </details>
          );
        })}
      </div>
      <div className="c3wa-pipeline-closed-piles">
        {collapsedStages.map((stage) => {
          const stageContacts = eligible.filter(
            (contact) => contact.stage === stage,
          );
          return (
            <details key={stage}>
              <summary>
                {WHATSAPP_STAGE_LABELS[stage]}{" "}
                <span>{stageContacts.length}</span>
              </summary>
              {stageContacts.map((contact) => (
                <PipelineCard
                  canEdit={canEdit && stage === "no_thanks"}
                  contact={contact}
                  customerStep={null}
                  hasDraft={draftIds.has(contact.waId)}
                  identity={resolveOutreachContactIdentity(
                    contact,
                    business.records,
                  )}
                  key={contact.waId}
                  onMove={(selectedContact, nextStage) =>
                    void move(selectedContact, nextStage)
                  }
                  onOpenChat={onOpenChat}
                  busy={busyWaId === contact.waId}
                />
              ))}
            </details>
          );
        })}
      </div>
    </main>
  );
}
