import * as React from "react";
import { UserRound } from "lucide-react";

import { Button } from "@/shared/ui/button";
import { cn } from "@/shared/lib/cn";
import { WhatsAppSendNowAction } from "./WhatsAppSendNowAction";
import {
  isWhatsAppPhoneLikeName,
  whatsappDeliveryLabel,
  type WhatsAppMessage,
  type WhatsAppTemplate,
} from "./whatsappOutreachModels";

export function whatsappInitials(value: string): string {
  return value
    .split(/\s+/u)
    .filter(Boolean)
    .slice(0, 2)
    .map((part) => part[0])
    .join("")
    .toUpperCase();
}

export function formatWhatsAppDateTime(value: string): string {
  const date = new Date(value);
  if (Number.isNaN(date.getTime())) return "";
  return new Intl.DateTimeFormat("en-ZA", {
    hour: "2-digit",
    minute: "2-digit",
    timeZone: "Africa/Johannesburg",
  }).format(date);
}

function isAgentAuthored(
  message: WhatsAppMessage,
  currentPubkey: string | undefined,
) {
  return Boolean(
    message.senderPubkey && message.senderPubkey !== currentPubkey,
  );
}

function deliveryGlyph(state: WhatsAppMessage["state"]): string {
  if (state === "read" || state === "delivered") return "✓✓";
  if (state === "sent") return "✓";
  if (state === "queued" || state === "sending") return "◷";
  if (state === "failed" || state === "unconfirmed") return "!";
  return "";
}

function ApprovalDraft({
  agentName,
  agentWritten,
  message,
  createdAt,
  onApprove,
  onReject,
}: {
  agentName: string;
  agentWritten: boolean;
  message: WhatsAppMessage;
  createdAt: string;
  onApprove: (text?: string) => Promise<void>;
  onReject: () => Promise<void>;
}) {
  const [editing, setEditing] = React.useState(false);
  const [text, setText] = React.useState(message.text);
  const [busy, setBusy] = React.useState(false);

  async function approve(value?: string) {
    setBusy(true);
    try {
      await onApprove(value);
      setEditing(false);
    } finally {
      setBusy(false);
    }
  }

  async function reject() {
    setBusy(true);
    try {
      await onReject();
    } finally {
      setBusy(false);
    }
  }

  return (
    <article
      className="c3wa-message-row is-outbound is-draft"
      data-testid={`whatsapp-draft-${message.id}`}
    >
      <span aria-hidden="true" className="c3wa-message-avatar is-sender">
        {whatsappInitials(agentName)}
      </span>
      <div className="c3wa-message-content">
        <div className="c3wa-message-author">
          <strong>{agentName}</strong>
          <span className="c3wa-agent-badge">AI</span>
          <time>{formatWhatsAppDateTime(createdAt)}</time>
        </div>
        <div className="c3wa-draft-card">
          <span className="c3wa-draft-pill">
            Draft by {agentWritten ? agentName : "Scout"}, needs your OK
          </span>
          {editing ? (
            <label
              className="c3wa-sr-only"
              htmlFor={`wa-draft-edit-${message.id}`}
            >
              Edit WhatsApp message draft
            </label>
          ) : null}
          {editing ? (
            <textarea
              data-testid="whatsapp-draft-edit"
              id={`wa-draft-edit-${message.id}`}
              onChange={(event) => setText(event.currentTarget.value)}
              rows={4}
              value={text}
            />
          ) : (
            <p>{message.text}</p>
          )}
          <div className="c3wa-draft-actions">
            {editing ? (
              <>
                <Button
                  disabled={busy || !text.trim()}
                  onClick={() => void approve(text.trim())}
                  size="sm"
                  type="button"
                >
                  Save and send
                </Button>
                <Button
                  disabled={busy}
                  onClick={() => {
                    setText(message.text);
                    setEditing(false);
                  }}
                  size="sm"
                  type="button"
                  variant="outline"
                >
                  Cancel
                </Button>
              </>
            ) : (
              <>
                <Button
                  disabled={busy}
                  onClick={() => void approve()}
                  size="sm"
                  type="button"
                >
                  Send
                </Button>
                <Button
                  disabled={busy}
                  onClick={() => setEditing(true)}
                  size="sm"
                  type="button"
                  variant="outline"
                >
                  Edit
                </Button>
                <Button
                  className="c3wa-text-danger"
                  disabled={busy}
                  onClick={() => void reject()}
                  size="sm"
                  type="button"
                  variant="outline"
                >
                  Discard
                </Button>
              </>
            )}
          </div>
        </div>
      </div>
    </article>
  );
}

export function WhatsAppThreadMessage({
  businessName,
  currentPubkey,
  currentUserName,
  agentName,
  message,
  templates,
  contactName,
  anonymousContact = false,
  waitingForApproval = false,
  canSendNow = false,
  sendNowBatchId = null,
  mobileHistory,
  onApproveDraft,
  onRejectDraft,
}: {
  businessName: string;
  currentPubkey: string | undefined;
  currentUserName: string;
  agentName: string;
  message: WhatsAppMessage;
  templates: readonly WhatsAppTemplate[];
  contactName: string;
  anonymousContact?: boolean;
  waitingForApproval?: boolean;
  canSendNow?: boolean;
  sendNowBatchId?: string | null;
  mobileHistory: boolean;
  onApproveDraft: (draftId: string, text?: string) => Promise<void>;
  onRejectDraft: (draftId: string) => Promise<void>;
}) {
  const agentWritten = isAgentAuthored(message, currentPubkey);
  if (message.state === "draft") {
    return (
      <ApprovalDraft
        agentName={agentName}
        agentWritten={agentWritten}
        createdAt={message.createdAt}
        message={message}
        onApprove={(text) => onApproveDraft(message.id, text)}
        onReject={() => onRejectDraft(message.id)}
      />
    );
  }
  const inbound = message.direction === "in";
  const statusLabel =
    message.state === null
      ? null
      : waitingForApproval
        ? "Waiting for your approval"
        : whatsappDeliveryLabel(
            message.state,
            message.errorText,
            message.sendsAt,
          );
  const template =
    templates.find((item) => item.name === message.templateName) ?? null;
  const mediaParts =
    message.kind === "media" ? message.text.split(/\s*·\s*/u) : [];
  return (
    <article
      className={cn(
        "c3wa-message-row",
        inbound ? "is-inbound" : "is-outbound",
        mobileHistory && "is-mobile-history",
      )}
      data-testid={`whatsapp-message-${message.id}`}
    >
      <span
        aria-hidden="true"
        className={cn(
          "c3wa-message-avatar",
          inbound ? "is-contact" : "is-sender",
        )}
      >
        {inbound &&
        (anonymousContact || isWhatsAppPhoneLikeName(contactName)) ? (
          <UserRound aria-hidden="true" size={16} strokeWidth={1.8} />
        ) : (
          whatsappInitials(
            inbound ? contactName : agentWritten ? agentName : currentUserName,
          )
        )}
      </span>
      <div className="c3wa-message-content">
        <div className="c3wa-message-author">
          <strong>
            {inbound ? contactName : agentWritten ? agentName : currentUserName}
          </strong>
          {!inbound && agentWritten ? (
            <span className="c3wa-agent-context">
              managed by {currentUserName} · sent as {businessName}
            </span>
          ) : null}
          <time>{formatWhatsAppDateTime(message.createdAt)}</time>
          {template ? (
            <span className="c3wa-template-marker">
              Template: <strong>{template.title}</strong>
            </span>
          ) : null}
        </div>
        {message.kind === "media" ? (
          <figure
            className="c3wa-media-card"
            aria-label={`Image attachment ${mediaParts[0] ?? message.text}`}
          >
            <span aria-hidden="true" className="c3wa-media-icon">
              ▧
            </span>
            <span>
              <strong>{mediaParts[0] ?? message.text}</strong>
              <small>{mediaParts.slice(1).join(" · ")}</small>
            </span>
          </figure>
        ) : message.kind === "button" ? (
          <div className="c3wa-button-reply">
            <p>
              <span aria-hidden="true">↶</span>
              {message.text}
            </p>
            <span>Tapped a reply button</span>
          </div>
        ) : (
          <p className="c3wa-message-text">{message.text}</p>
        )}
        {template?.buttons.length ? (
          <fieldset className="c3wa-template-buttons">
            <legend className="sr-only">Reply buttons the contact sees</legend>
            {template.buttons.map((button) => (
              <span key={`${button.type}:${button.text}`}>↶ {button.text}</span>
            ))}
          </fieldset>
        ) : null}
        {message.state === "failed" ? (
          <div className="c3wa-message-failure" role="status">
            <span aria-hidden="true">!</span>
            <span>{message.errorText ?? statusLabel}</span>
          </div>
        ) : !inbound && message.state !== null ? (
          <div className="c3wa-message-receipt">
            <span aria-hidden="true" className="c3wa-delivery-glyph">
              {deliveryGlyph(message.state)}
            </span>
            {agentWritten ? (
              <>
                <span>{statusLabel}</span>
                {waitingForApproval ? null : (
                  <span className="c3wa-approved-mark">
                    Approved by {currentUserName}
                    {message.kind === "template" ? " in a batch" : ""}
                  </span>
                )}
              </>
            ) : (
              <span>
                {statusLabel} · you, as {businessName}
              </span>
            )}
            {canSendNow && message.state === "queued" && sendNowBatchId ? (
              <WhatsAppSendNowAction batchId={sendNowBatchId} size="xs" />
            ) : null}
          </div>
        ) : null}
      </div>
    </article>
  );
}
