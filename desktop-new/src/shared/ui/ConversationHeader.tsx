import type { ReactNode } from "react";

/**
 * The orientation row for a message-bearing conversation surface.
 *
 * This is presentation only: the owning feature resolves whether the
 * conversation is a channel, direct message, or session, and supplies its
 * identity, status, and actions.
 */
export function ConversationHeader({
  title,
  icon,
  metadata,
  context,
  actions,
}: {
  title: string;
  /** Decorative conversation identity, such as a channel glyph or avatar. */
  icon?: ReactNode;
  /** One short status associated with the title, such as “On this device”. */
  metadata?: ReactNode;
  /** Optional, feature-owned context when the title alone is ambiguous. */
  context?: ReactNode;
  /** Feature-owned controls, ordered by that feature's action priority. */
  actions?: ReactNode;
}) {
  return (
    <header className="conversation-header">
      <div className="conversation-header-identity">
        {icon ? (
          <span className="conversation-header-icon" aria-hidden="true">
            {icon}
          </span>
        ) : null}
        <div className="conversation-header-title-block">
          <div className="conversation-header-title-row">
            <h1 className="text-heading text-primary">{title}</h1>
            {metadata ? (
              <span className="conversation-header-metadata">{metadata}</span>
            ) : null}
          </div>
          {context ? (
            <div className="conversation-header-context text-body-sm text-secondary">
              {context}
            </div>
          ) : null}
        </div>
      </div>
      {actions ? (
        <div className="conversation-header-actions">{actions}</div>
      ) : null}
    </header>
  );
}
