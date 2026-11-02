import * as React from "react";
import { useNavigate } from "@tanstack/react-router";
import { useChannelsQuery } from "@/features/channels/hooks";
import { Button } from "@/shared/ui/button";
import { cn } from "@/shared/lib/cn";
import { useMyRelayMembershipLookupQuery } from "@/features/community-members/hooks";
import { useKnownAgentPubkeys } from "@/features/agents/useKnownAgentPubkeys";
import { MessageComposer } from "@/features/messages/ui/MessageComposer";
import { useSendMessageMutation } from "@/features/messages/hooks";
import { ScoutAvatar } from "@/features/onboarding/ui/ScoutAvatar";
import { useIdentityQuery } from "@/shared/api/hooks";
import { normalizePubkey } from "@/shared/lib/pubkey";
import { useProfileQuery, useUsersBatchQuery } from "@/features/profile/hooks";
import { UserAvatar } from "@/shared/ui/UserAvatar";
import { KIND_WHATSAPP_BATCH_DECISION } from "@/shared/constants/kinds";
import {
  maskWhatsAppNumber,
  renderWhatsAppTemplate,
  whatsappQueuedScheduleLabel,
  whatsappTemplateDisplayTitle,
  type WhatsAppBatchRow,
  type WhatsAppContact,
  type WhatsAppTemplate,
} from "./whatsappOutreachModels";
import { WhatsAppSendNowAction } from "./WhatsAppSendNowAction";
import {
  canSendNowWhatsAppBatch,
  canUseWhatsAppSendNow,
} from "./whatsappSendNowEligibility";
import {
  useWhatsAppBatchQuery,
  useWhatsAppCommand,
  useWhatsAppContactsQuery,
  useWhatsAppStatusQuery,
  useWhatsAppTemplatesQuery,
} from "./whatsappOutreachQueries";
import { findWhatsAppBatchCreatorChannel } from "./whatsappBatchNavigation";
import "./whatsappOutreach.css";

function formatTime(value: string | null): string | null {
  if (!value) return null;
  const date = new Date(value);
  if (Number.isNaN(date.getTime())) return null;
  return new Intl.DateTimeFormat("en-ZA", {
    hour: "2-digit",
    minute: "2-digit",
    timeZone: "Africa/Johannesburg",
  }).format(date);
}

function resolveTemplate(
  nameOrTitle: string,
  templates: readonly WhatsAppTemplate[],
): WhatsAppTemplate | null {
  return (
    templates.find(
      (template) =>
        template.name === nameOrTitle || template.title === nameOrTitle,
    ) ?? null
  );
}

function displayTemplateCategory(category: string): string {
  return (
    category.charAt(0).toLocaleUpperCase() +
    category.slice(1).toLocaleLowerCase()
  );
}

function contactForRow(
  row: WhatsAppBatchRow,
  contacts: readonly WhatsAppContact[],
): WhatsAppContact | null {
  const digits = row.to.replace(/\D/gu, "");
  return (
    contacts.find(
      (contact) =>
        contact.e164.replace(/\D/gu, "") === digits || contact.waId === digits,
    ) ?? null
  );
}

function rolledByCap(row: WhatsAppBatchRow): boolean {
  return (
    row.state === "held" ||
    row.skipReason?.toLocaleLowerCase().includes("cap") === true
  );
}

function TemplatePreview({
  template,
  params,
}: {
  template: WhatsAppTemplate;
  params: readonly string[];
}) {
  const segments = template.body.split(/(\{\{\d+\}\})/gu);
  let position = 0;
  const keyedSegments = segments.map((segment) => {
    const keyedSegment = { key: `${position}:${segment}`, segment };
    position += segment.length;
    return keyedSegment;
  });
  return (
    <>
      {keyedSegments.map(({ key, segment }) => {
        const slot = /^\{\{(\d+)\}\}$/u.exec(segment);
        if (!slot) return <React.Fragment key={key}>{segment}</React.Fragment>;
        const value = params[Number(slot[1]) - 1] ?? segment;
        if (/^https:\/\//u.test(value)) {
          const previewLink = value
            .replace(/^https:\/\//u, "")
            .replace(/\/+$/u, "");
          return (
            <a
              className="c3wa-preview-link"
              href={value}
              key={key}
              rel="noreferrer noopener"
              target="_blank"
            >
              {previewLink}
            </a>
          );
        }
        return (
          <mark className="c3wa-variable-highlight" key={key}>
            {value}
          </mark>
        );
      })}
    </>
  );
}

function BatchRow({
  contact,
  leadName,
  materialLink,
  index,
  isNext,
  isRemoved,
  onRemove,
  onSave,
  params,
  preview,
  row,
  template,
  testIdPrefix,
  editable,
}: {
  contact: WhatsAppContact | null;
  leadName: string | null;
  materialLink: string | null;
  index: number;
  isNext: boolean;
  isRemoved: boolean;
  onRemove: () => void;
  onSave: (params: string[]) => void;
  params: readonly string[];
  preview: string;
  row: WhatsAppBatchRow;
  template: WhatsAppTemplate | null;
  testIdPrefix: string;
  editable: boolean;
}) {
  const [editing, setEditing] = React.useState(false);
  const [values, setValues] = React.useState<string[]>([...params]);
  React.useEffect(() => setValues([...params]), [params]);

  function saveEdit() {
    onSave(values);
    setEditing(false);
  }

  return (
    <article
      className={cn("c3wa-batch-row", isRemoved && "is-removed")}
      data-row-tone={index % 5}
      data-testid={`${testIdPrefix}-${row.idx}`}
    >
      <div className="c3wa-batch-row-head">
        <span aria-hidden="true" className="c3wa-batch-row-art">
          <i />
          <i />
          <i />
        </span>
        <div className="c3wa-batch-row-contact">
          <strong>
            {contact?.businessName ??
              leadName ??
              contact?.displayName ??
              (row.leadId ? "Linked lead" : `WhatsApp contact ${index + 1}`)}
          </strong>
          <span>{maskWhatsAppNumber(row.to)}</span>
        </div>
        {materialLink ? (
          <a
            className="c3wa-row-preview-link"
            href={materialLink}
            rel="noreferrer noopener"
            target="_blank"
          >
            Website preview ↗
          </a>
        ) : null}
        {row.state === "sent" ||
        row.state === "delivered" ||
        row.state === "read" ? (
          <span
            className={cn(
              "c3wa-batch-row-state",
              row.state === "read"
                ? "is-read"
                : row.state === "delivered"
                  ? "is-delivered"
                  : "is-sent",
            )}
          >
            {row.state === "read"
              ? "Read"
              : row.state === "delivered"
                ? "Delivered"
                : "Sent"}
          </span>
        ) : null}
        {rolledByCap(row) ? (
          <span className="c3wa-batch-row-state is-held">Tomorrow</span>
        ) : null}
        {row.state === "failed" ? (
          <span className="c3wa-batch-row-state is-failed">Failed</span>
        ) : null}
        {row.state === "ready" || row.state === "queued" ? (
          <span
            className="c3wa-batch-row-state"
            data-testid={`whatsapp-batch-row-schedule-${row.idx}`}
          >
            {whatsappQueuedScheduleLabel(row.sendsAt)}
          </span>
        ) : null}
        {isNext ? (
          <span className="c3wa-batch-row-state is-next">Up next</span>
        ) : null}
        {editable && !isRemoved ? (
          <div className="c3wa-batch-row-actions">
            <button
              onClick={() => setEditing((current) => !current)}
              type="button"
            >
              {editing ? "Cancel edit" : "Edit"}
            </button>
            <button onClick={onRemove} type="button">
              Remove
            </button>
          </div>
        ) : null}
      </div>
      {editing ? (
        <div className="c3wa-batch-edit-fields">
          {values
            .map((value, slotIndex) => ({
              key: `${row.idx}:variable:${slotIndex + 1}`,
              slotIndex,
              value,
            }))
            .map(({ key, slotIndex, value }) => (
              <label key={key}>
                <span>Variable {slotIndex + 1}</span>
                <input
                  aria-label={`Row ${index + 1} variable ${slotIndex + 1}`}
                  onChange={(event) => {
                    const value = event.currentTarget.value;
                    setValues((current) =>
                      current.map((item, itemIndex) =>
                        itemIndex === slotIndex ? value : item,
                      ),
                    );
                  }}
                  value={value}
                />
              </label>
            ))}
          <Button onClick={saveEdit} size="sm" type="button">
            Save edit
          </Button>
          <p aria-live="polite" className="c3wa-batch-preview">
            {template ? (
              <TemplatePreview params={values} template={template} />
            ) : (
              preview
            )}
          </p>
        </div>
      ) : null}
      {row.skipReason ? (
        <p className="c3wa-batch-skip">{row.skipReason}</p>
      ) : null}
      {isRemoved ? (
        <p className="c3wa-batch-removed">
          This row will be removed when you approve.
        </p>
      ) : null}
    </article>
  );
}

export function WhatsAppBatchScreen({
  batchId,
  channelId = null,
  redirectToChat = false,
}: {
  batchId: string;
  channelId?: string | null;
  redirectToChat?: boolean;
}) {
  const navigate = useNavigate();
  const batchQuery = useWhatsAppBatchQuery(batchId);
  const statusQuery = useWhatsAppStatusQuery();
  const contactsQuery = useWhatsAppContactsQuery();
  const templatesQuery = useWhatsAppTemplatesQuery(
    statusQuery.data?.connected === true,
  );
  const identityQuery = useIdentityQuery();
  const profileQuery = useProfileQuery();
  const currentPubkey = identityQuery.data?.pubkey;
  const currentUserProfilesQuery = useUsersBatchQuery(
    currentPubkey ? [currentPubkey] : [],
    { enabled: Boolean(currentPubkey) },
  );
  const channelsQuery = useChannelsQuery();
  const membershipQuery = useMyRelayMembershipLookupQuery();
  const knownAgentPubkeys = useKnownAgentPubkeys();
  const command = useWhatsAppCommand();
  const [removedRows, setRemovedRows] = React.useState<number[]>([]);
  const [editedRows, setEditedRows] = React.useState<
    Record<number, readonly string[]>
  >({});
  const [expanded, setExpanded] = React.useState(false);
  const [busy, setBusy] = React.useState(false);
  const [error, setError] = React.useState("");

  const batch = batchQuery.data;
  const contacts = contactsQuery.data ?? [];
  const templates = templatesQuery.data ?? statusQuery.data?.templates ?? [];
  const template = batch ? resolveTemplate(batch.template, templates) : null;
  const rows = batch?.rows ?? [];
  const pending = batch?.status === "pending";
  const approved =
    batch?.status === "approved" || batch?.status === "completed";
  const rejected = batch?.status === "rejected";
  const role = membershipQuery.data?.membership?.role;
  const canManage =
    membershipQuery.data?.membershipRequired === false ||
    role === "owner" ||
    role === "admin";
  const currentUserProfile = currentPubkey
    ? currentUserProfilesQuery.data?.profiles[normalizePubkey(currentPubkey)]
    : undefined;
  const canSendNow = canUseWhatsAppSendNow({
    role,
    pubkey: profileQuery.isSuccess ? currentPubkey : undefined,
    isAgent: currentPubkey
      ? knownAgentPubkeys.has(normalizePubkey(currentPubkey)) ||
        currentUserProfile?.isAgent === true
      : false,
    ownerPubkey:
      profileQuery.data?.ownerPubkey ?? currentUserProfile?.ownerPubkey,
    knownAgentPubkeys,
  });
  const scoutChannel = React.useMemo(() => {
    if (!batch) return null;
    return findWhatsAppBatchCreatorChannel(
      channelsQuery.data ?? [],
      batch.createdBy,
      currentPubkey,
    );
  }, [batch, channelsQuery.data, currentPubkey]);
  const sendMessage = useSendMessageMutation(scoutChannel, identityQuery.data);
  React.useEffect(() => {
    if (!redirectToChat || !scoutChannel) return;
    void navigate({
      to: "/channels/$channelId",
      params: { channelId: scoutChannel.id },
      search: { whatsappBatchId: batchId },
      replace: true,
    });
  }, [batchId, navigate, redirectToChat, scoutChannel]);
  const decidedByCurrentUser = Boolean(
    batch?.decidedBy && batch.decidedBy === currentPubkey,
  );
  const visibleRows = expanded ? rows : rows.slice(0, pending ? 5 : 8);
  const nextReadyRowIdx = approved
    ? (rows.find((row) => row.state === "ready")?.idx ?? null)
    : null;
  const relayRolledCount = rows.filter(rolledByCap).length;
  const capAvailable = Math.max(
    0,
    (statusQuery.data?.dailyNewChatCap ?? 0) -
      (statusQuery.data?.newChatsSentToday ?? 0),
  );
  const estimatedPendingRoll = pending
    ? Math.max(0, rows.length - capAvailable)
    : 0;
  const rolledCount = Math.max(relayRolledCount, estimatedPendingRoll);
  const nextSend = batch?.nextSendAt ?? null;
  const firstRow = rows[0] ?? null;
  const firstRowContact = firstRow ? contactForRow(firstRow, contacts) : null;
  const firstRowParams = firstRow
    ? (editedRows[firstRow.idx] ?? firstRow.params)
    : [];
  const readyToday = rows.filter((row) =>
    ["ready", "queued", "sending"].includes(row.state),
  ).length;
  const remainingToday = pending
    ? Math.min(rows.length, capAvailable)
    : readyToday;
  const canSendNowBatch = canSendNowWhatsAppBatch(batch, canSendNow);

  function updateRowParams(idx: number, params: readonly string[]) {
    setEditedRows((current) => ({ ...current, [idx]: params }));
  }

  function toggleRemoved(idx: number) {
    setRemovedRows((current) =>
      current.includes(idx)
        ? current.filter((value) => value !== idx)
        : [...current, idx],
    );
  }

  async function decide(decision: "approve" | "reject") {
    if (!batch || !canManage || busy) return;
    setBusy(true);
    setError("");
    try {
      await command(KIND_WHATSAPP_BATCH_DECISION, {
        v: 1,
        batchId: batch.batchId,
        decision,
        ...(decision === "approve"
          ? {
              dropRows: [...removedRows].sort((a, b) => a - b),
              editRows: Object.entries(editedRows)
                .filter(([idx]) => !removedRows.includes(Number(idx)))
                .map(([idx, params]) => ({ idx: Number(idx), params })),
            }
          : {}),
      });
    } catch (decisionError) {
      setError(
        decisionError instanceof Error
          ? decisionError.message
          : "Could not update this batch.",
      );
    } finally {
      setBusy(false);
    }
  }

  function openOutreach() {
    const channelId = statusQuery.data?.outreachChannelId;
    if (!channelId) return;
    void navigate({ to: "/channels/$channelId", params: { channelId } });
  }

  function returnToScout() {
    if (scoutChannel) {
      void navigate({
        to: "/channels/$channelId",
        params: { channelId: scoutChannel.id },
      });
      return;
    }
    void navigate({ to: "/" });
  }

  async function sendScoutMessage(
    content: string,
    mentionPubkeys: string[],
    mediaTags?: string[][],
    channelId?: string | null,
  ) {
    if (!scoutChannel) return;
    await sendMessage.mutateAsync({
      channelId: channelId ?? scoutChannel.id,
      content,
      mentionPubkeys,
      mediaTags,
    });
  }

  if (batchQuery.isPending) {
    return (
      <div aria-busy="true" className="c3wa-loading c3wa-batch-loading">
        Loading batch…
      </div>
    );
  }
  if (batchQuery.isError) {
    return (
      <main className="c3wa-root c3wa-batch-error" role="alert">
        {batchQuery.error.message}
      </main>
    );
  }
  if (!batch) return null;
  if (redirectToChat && scoutChannel)
    return (
      <div aria-busy="true" className="c3wa-loading c3wa-batch-loading">
        Opening Scout’s chat…
      </div>
    );
  if (channelId && scoutChannel && scoutChannel.id !== channelId) {
    return (
      <main className="c3wa-root c3wa-batch-error" role="alert">
        This batch belongs in Scout’s chat.
      </main>
    );
  }

  const total = batch.counts.total || rows.length;
  const sent = batch.counts.sent;
  const allDone =
    batch.status === "completed" ||
    (batch.status === "approved" &&
      batch.counts.ready === 0 &&
      sent + batch.counts.failed + batch.counts.dropped >= total);
  const progressPercent =
    total > 0 ? Math.min(100, Math.round((sent / total) * 100)) : 0;

  return (
    <main
      className="c3wa-root c3wa-batch-page"
      data-testid="whatsapp-batch-page"
    >
      <section
        className="c3wa-mobile-batch"
        data-testid="whatsapp-mobile-batch"
      >
        <div aria-hidden="true" className="c3wa-mobile-safe-area" />
        <header className="c3wa-mobile-batch-header">
          <button
            aria-label="Back to Scout chat"
            onClick={returnToScout}
            type="button"
          >
            ‹
          </button>
          <span>
            <strong>WhatsApp batch</strong>
            <small>
              From Scout · today {formatTime(batch.createdAt) ?? "10:46"}
            </small>
          </span>
          <button
            aria-label="WhatsApp batch details"
            onClick={openOutreach}
            type="button"
          >
            ⋮
          </button>
        </header>
        <div className="c3wa-mobile-batch-scroll">
          <span
            className={cn(
              "c3wa-mobile-batch-status",
              pending
                ? "is-needs-you"
                : approved
                  ? "is-approved"
                  : "is-rejected",
            )}
          >
            {pending
              ? "Your approval is needed"
              : approved
                ? "Batch approved"
                : "Batch rejected"}
          </span>
          <h1>
            {pending
              ? `${total} businesses`
              : approved
                ? `Messaging ${total} businesses`
                : "Batch rejected"}
          </h1>
          {pending ? (
            <>
              <p className="c3wa-mobile-batch-intro">
                Scout wants to message them on WhatsApp, each with a website
                preview. Nothing is sent until you approve.
              </p>
              <div className="c3wa-mobile-first-message">
                <strong>First message</strong>
                {template ? (
                  <>
                    <div className="c3wa-mobile-first-preview">
                      <TemplatePreview
                        params={firstRowParams}
                        template={template}
                      />
                    </div>
                    <div className="c3wa-template-buttons">
                      {template.buttons.map((button) => (
                        <span key={`${button.type}:${button.text}`}>
                          ↶ {button.text}
                        </span>
                      ))}
                    </div>
                    <p className="c3wa-mobile-template-label">
                      Template: {whatsappTemplateDisplayTitle(template)}.{" "}
                      {displayTemplateCategory(template.category)}, approved by
                      Meta. The preview link is filled in per contact.
                    </p>
                  </>
                ) : (
                  <p>Approved WhatsApp template</p>
                )}
              </div>
            </>
          ) : rejected ? (
            <p className="c3wa-mobile-batch-intro">
              This request was rejected. No messages were sent.
            </p>
          ) : null}
          <div className="c3wa-mobile-batch-rows">
            <div className="c3wa-batch-row-heading">
              <strong>Guesthouses · {rows.length}</strong>
            </div>
            {visibleRows.map((row, index) => (
              <BatchRow
                contact={contactForRow(row, contacts)}
                editable={pending && canManage}
                index={index}
                isNext={row.idx === nextReadyRowIdx}
                isRemoved={removedRows.includes(row.idx)}
                key={`mobile:${row.idx}`}
                leadName={null}
                materialLink={batch.materialLinks[index] ?? null}
                onRemove={() => toggleRemoved(row.idx)}
                onSave={(params) => updateRowParams(row.idx, params)}
                params={editedRows[row.idx] ?? row.params}
                preview={
                  editedRows[row.idx] && template
                    ? renderWhatsAppTemplate(template.body, editedRows[row.idx])
                    : row.preview
                }
                row={row}
                template={template}
                testIdPrefix="whatsapp-mobile-batch-row"
              />
            ))}
            {rows.length > 5 ? (
              <button
                className="c3wa-batch-show-all"
                onClick={() => setExpanded((current) => !current)}
                type="button"
              >
                {expanded ? "Show fewer" : `Show all ${rows.length}`}
              </button>
            ) : null}
          </div>
          {pending ? (
            <div className="c3wa-cap-summary">
              <span aria-hidden="true">◷</span>
              <p>
                {statusQuery.data?.newChatsSentToday ?? 0} of{" "}
                {statusQuery.data?.dailyNewChatCap ?? 30} new chats used today.{" "}
                {remainingToday} send today, {rolledCount} roll to tomorrow.
                Replies are never capped.
              </p>
            </div>
          ) : approved ? (
            <div className="c3wa-batch-progress">
              <div className="c3wa-batch-progress-copy">
                <strong>
                  {sent} of {total} sent
                </strong>
                <span>
                  {decidedByCurrentUser
                    ? `Approved by ${profileQuery.data?.displayName ?? "you"}`
                    : "Approved by owner or admin"}
                </span>
              </div>
              <div
                className="c3wa-progress-track"
                role="progressbar"
                aria-label="Batch messages sent"
                aria-valuemax={total}
                aria-valuemin={0}
                aria-valuenow={sent}
              >
                <span style={{ width: `${progressPercent}%` }} />
              </div>
              {!allDone ? (
                <p>
                  {nextSend
                    ? `Next send at ${formatTime(nextSend)} South African time. ${remainingToday} still to send today.`
                    : "Next send follows the relay's South African pacing window, from 08:00 to 18:00."}
                </p>
              ) : (
                <p>All sendable messages in this batch have finished.</p>
              )}
              {rolledCount ? (
                <p>
                  {rolledCount} {rolledCount === 1 ? "row" : "rows"} rolled to
                  tomorrow by the daily cap.
                </p>
              ) : null}
              {canSendNowBatch ? (
                <div className="c3wa-send-now-action">
                  <WhatsAppSendNowAction batchId={batch.batchId} />
                </div>
              ) : null}
            </div>
          ) : (
            <p className="c3wa-batch-rejected-note">
              This request was rejected. No messages were sent.
            </p>
          )}
          {error ? (
            <div className="c3wa-alert c3wa-batch-alert" role="alert">
              {error}
            </div>
          ) : null}
        </div>
        {pending ? (
          <footer className="c3wa-mobile-batch-actions">
            <Button
              disabled={!canManage || busy}
              onClick={() => void decide("reject")}
              type="button"
              variant="outline"
            >
              Reject
            </Button>
            <Button
              disabled={!canManage || busy}
              onClick={() => void decide("approve")}
              type="button"
            >
              {busy
                ? "Approving…"
                : `Approve all ${rows.length - removedRows.length}`}
            </Button>
          </footer>
        ) : null}
      </section>

      <header className="c3wa-scout-head">
        <span className="c3wa-scout-avatar-shell">
          <ScoutAvatar className="c3wa-scout-avatar" pose="hello" />
        </span>
        <div>
          <strong>Scout</strong>
          <span>
            AI employee · managed by{" "}
            {profileQuery.data?.displayName ?? "the owner"}
          </span>
        </div>
        <div className="c3wa-scout-head-actions" aria-hidden="true">
          <span>▣</span>
          <span>⌕</span>
          <span>⋮</span>
        </div>
      </header>

      <div className="c3wa-batch-history">
        <section className="c3wa-batch-layout">
          <div className="c3wa-batch-day-divider">
            <span>Today</span>
          </div>
          <div className="c3wa-owner-message">
            <span aria-hidden="true" className="c3wa-owner-message-avatar">
              <UserAvatar
                avatarUrl={profileQuery.data?.avatarUrl ?? null}
                className="size-full"
                displayName={profileQuery.data?.displayName ?? "Owner"}
                fallbackDelayMs={0}
                pubkey={currentPubkey}
              />
            </span>
            <div>
              <div className="c3wa-owner-message-meta">
                <strong>{profileQuery.data?.displayName ?? "You"}</strong>
                <span>10:31</span>
              </div>
              <p>
                Prepare these contacts for WhatsApp. Use the selected template.
              </p>
            </div>
          </div>
          <div className="c3wa-scout-message">
            <span className="c3wa-scout-message-avatar-shell">
              <ScoutAvatar className="c3wa-scout-message-avatar" pose="hello" />
            </span>
            <div className="c3wa-scout-message-body">
              <div className="c3wa-scout-message-meta">
                <strong>Scout</strong>
                <span>
                  managed by {profileQuery.data?.displayName ?? "the owner"} ·{" "}
                  {formatTime(batch.createdAt) ?? "Today"}
                </span>
              </div>
              <p>
                {total} are ready, each with a website preview. Nothing goes out
                until you approve.
              </p>
              <div className="c3wa-batch-card">
                <header className="c3wa-batch-card-head">
                  <div className="c3wa-batch-status-pills">
                    <span
                      className={cn(
                        "c3wa-batch-flag",
                        pending
                          ? "is-needs-you"
                          : approved
                            ? "is-approved"
                            : "is-rejected",
                      )}
                    >
                      {pending
                        ? "Needs you"
                        : approved
                          ? "Approved"
                          : "Rejected"}
                    </span>
                    <span className="c3wa-batch-type-pill">
                      <span className="c3wa-whatsapp-glyph" aria-hidden="true">
                        W
                      </span>
                      WhatsApp batch
                    </span>
                    {pending ? (
                      <span className="c3wa-batch-sender">
                        Sends from{" "}
                        {statusQuery.data?.displayPhoneNumber ??
                          "business number"}
                      </span>
                    ) : approved ? (
                      <span className="c3wa-batch-sender">
                        {decidedByCurrentUser
                          ? "You approved at"
                          : "Approved at"}{" "}
                        {formatTime(batch.decidedAt) ?? "today"} ·{" "}
                        {template
                          ? whatsappTemplateDisplayTitle(template)
                          : batch.template}
                      </span>
                    ) : (
                      <span className="c3wa-batch-sender">
                        Rejected at {formatTime(batch.decidedAt) ?? "today"} ·{" "}
                        {template
                          ? whatsappTemplateDisplayTitle(template)
                          : batch.template}
                      </span>
                    )}
                  </div>
                </header>
                {template ? (
                  <div
                    className={cn(
                      "c3wa-batch-template",
                      approved ? "is-approved" : rejected && "is-rejected",
                    )}
                  >
                    {pending ? (
                      <>
                        <h2>Scout wants to message {total} businesses</h2>
                        <p className="c3wa-batch-template-label">
                          Template: {whatsappTemplateDisplayTitle(template)} ·{" "}
                          {displayTemplateCategory(template.category)} ·
                          approved by Meta
                        </p>
                        <div className="c3wa-batch-first-preview">
                          <strong>
                            First message, as{" "}
                            {firstRowContact?.displayName ??
                              "the first contact"}{" "}
                            will see it
                          </strong>
                          <p>
                            <TemplatePreview
                              params={firstRowParams}
                              template={template}
                            />
                          </p>
                          <div className="c3wa-template-buttons">
                            {template.buttons.map((button) => (
                              <span key={`${button.type}:${button.text}`}>
                                ↶ {button.text}
                              </span>
                            ))}
                          </div>
                        </div>
                      </>
                    ) : approved ? (
                      <>
                        <h2>Messaging {total} businesses</h2>
                        <div
                          className="c3wa-batch-progress is-approved"
                          data-testid="whatsapp-batch-progress"
                        >
                          <div className="c3wa-batch-progress-copy">
                            <strong>
                              {sent} <span>of {total} sent</span>
                            </strong>
                          </div>
                          <div
                            className="c3wa-progress-track is-segmented"
                            role="progressbar"
                            aria-label="Batch sent"
                            aria-valuemax={total}
                            aria-valuemin={0}
                            aria-valuenow={sent}
                          >
                            <span
                              aria-hidden="true"
                              className="c3wa-progress-sent"
                              style={{ flexGrow: sent, flexBasis: 0 }}
                            />
                            <span
                              aria-hidden="true"
                              className="c3wa-progress-ready"
                              style={{ flexGrow: remainingToday, flexBasis: 0 }}
                            />
                            <span
                              aria-hidden="true"
                              className="c3wa-progress-held"
                              style={{ flexGrow: rolledCount, flexBasis: 0 }}
                            />
                          </div>
                          <div className="c3wa-batch-progress-stats">
                            <div>
                              <span>Next send</span>
                              <strong>
                                {formatTime(nextSend) ?? "Scheduled"}
                              </strong>
                            </div>
                            <div>
                              <span>Still to go today</span>
                              <strong>{remainingToday}</strong>
                            </div>
                            <div className="is-held">
                              <span>Roll to tomorrow</span>
                              <strong>{rolledCount}</strong>
                            </div>
                          </div>
                          <p>
                            Today’s cap of{" "}
                            {statusQuery.data?.dailyNewChatCap ?? 30} new chats
                            is reached after {remainingToday} more. Sends are
                            spaced about 8 minutes apart to keep the number’s
                            quality Green.
                          </p>
                          {canSendNowBatch ? (
                            <div className="c3wa-send-now-action">
                              <WhatsAppSendNowAction batchId={batch.batchId} />
                            </div>
                          ) : null}
                        </div>
                      </>
                    ) : (
                      <>
                        <h2>Batch rejected</h2>
                        <p>No messages were sent.</p>
                      </>
                    )}
                  </div>
                ) : (
                  <div className="c3wa-batch-template">
                    <strong>WhatsApp template</strong>
                    <span>Approved message template</span>
                  </div>
                )}
                <div className="c3wa-batch-row-heading">
                  <strong>Guesthouses · {rows.length}</strong>
                  <span>Preview link filled in per guesthouse</span>
                </div>
                <div className="c3wa-batch-row-list">
                  {visibleRows.map((row, index) => (
                    <BatchRow
                      contact={contactForRow(row, contacts)}
                      editable={pending && canManage}
                      index={index}
                      isNext={row.idx === nextReadyRowIdx}
                      isRemoved={removedRows.includes(row.idx)}
                      key={row.idx}
                      leadName={null}
                      materialLink={batch.materialLinks[index] ?? null}
                      onRemove={() => toggleRemoved(row.idx)}
                      onSave={(params) => updateRowParams(row.idx, params)}
                      params={editedRows[row.idx] ?? row.params}
                      preview={
                        editedRows[row.idx] && template
                          ? renderWhatsAppTemplate(
                              template.body,
                              editedRows[row.idx],
                            )
                          : row.preview
                      }
                      row={row}
                      template={template}
                      testIdPrefix="whatsapp-batch-row"
                    />
                  ))}
                  {rows.length > 5 ? (
                    <button
                      className="c3wa-batch-show-all"
                      onClick={() => setExpanded((current) => !current)}
                      type="button"
                    >
                      {expanded ? "Show fewer" : `Show all ${rows.length}`}
                    </button>
                  ) : null}
                </div>
                {pending ? (
                  <div className="c3wa-cap-summary">
                    <span aria-hidden="true">◷</span>
                    <p>
                      {statusQuery.data?.newChatsSentToday ?? 0} of{" "}
                      {statusQuery.data?.dailyNewChatCap ?? 30} new chats used
                      today. {remainingToday} send today, {rolledCount} roll to
                      tomorrow. Replies are never capped.
                    </p>
                  </div>
                ) : null}
                {error ? (
                  <div className="c3wa-alert c3wa-batch-alert" role="alert">
                    {error}
                  </div>
                ) : null}
                {pending ? (
                  <div className="c3wa-batch-actions">
                    <Button
                      disabled={!canManage || busy}
                      onClick={() => void decide("approve")}
                      type="button"
                    >
                      {busy
                        ? "Approving…"
                        : `Approve all ${rows.length - removedRows.length}`}
                    </Button>
                    <Button
                      className="c3wa-reject-batch"
                      disabled={!canManage || busy}
                      onClick={() => void decide("reject")}
                      type="button"
                      variant="outline"
                    >
                      Reject
                    </Button>
                  </div>
                ) : (
                  <div className="c3wa-batch-actions c3wa-batch-actions-progress">
                    <Button
                      onClick={openOutreach}
                      type="button"
                      variant="outline"
                    >
                      Open #Outreach
                    </Button>
                  </div>
                )}
              </div>
            </div>
          </div>
          {approved ? (
            <div className="c3wa-scout-message c3wa-batch-follow-up">
              <span className="c3wa-scout-message-avatar-shell">
                <ScoutAvatar
                  className="c3wa-scout-message-avatar"
                  pose="hello"
                />
              </span>
              <div>
                <div className="c3wa-scout-message-meta">
                  <strong>Scout</strong>
                  <span>
                    managed by {profileQuery.data?.displayName ?? "the owner"} ·
                    11:34
                  </span>
                </div>
                <p>
                  The approved batch is ready. Replies land in{" "}
                  <button onClick={openOutreach} type="button">
                    #Outreach
                  </button>
                  .
                </p>
              </div>
            </div>
          ) : null}
        </section>
      </div>
      <footer className="c3wa-batch-chat-composer">
        <MessageComposer
          channelId={scoutChannel?.id ?? null}
          channelName="Scout"
          channelType="dm"
          disabled={!scoutChannel?.isMember || sendMessage.isPending}
          layoutMode="standalone"
          onSend={sendScoutMessage}
          placeholder="Message Scout"
          showLeadReference={false}
          toolbarLayout="dm"
        />
      </footer>
    </main>
  );
}
