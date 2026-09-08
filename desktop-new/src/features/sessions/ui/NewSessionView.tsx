import { IconArrowLeft, IconHash, IconLock } from "@tabler/icons-react";
import { MessageComposer } from "@/features/composer/ui/MessageComposer";
import type { Channel, Message } from "../types";

export function NewSessionView({
  origin,
  pending,
  error,
  onBack,
  onCreate,
  draft,
  onDraftChange,
}: {
  origin?: Channel;
  pending?: Message;
  error?: string | null;
  onBack: () => void;
  onCreate: (content: string) => Promise<void>;
  draft: string;
  onDraftChange: (value: string) => void;
}) {
  return (
    <main className="session-view new-session">
      <header className="session-header">
        <button
          type="button"
          className="icon-button"
          onClick={onBack}
          aria-label="Back"
        >
          <IconArrowLeft size={19} stroke={1.6} aria-hidden="true" />
        </button>
        <div className="session-title-block">
          <h1 className="text-body text-primary">New Session</h1>
          {origin ? (
            <span className="origin-label text-body-sm text-secondary">
              <IconHash size={13} stroke={1.6} aria-hidden="true" />
              From {origin.name}
            </span>
          ) : (
            <span className="origin-label text-body-sm text-secondary">
              <IconLock size={13} stroke={1.6} aria-hidden="true" />
              Only you
            </span>
          )}
        </div>
      </header>
      <div className="new-session-center">
        <div className="new-session-intro">
          <span className="session-orb" aria-hidden="true" />
          <h2 className="text-heading text-primary">
            What are you working on?
          </h2>
          <p className="text-body text-secondary">
            Start with a message. You can bring in people and agents when the
            work needs them.
          </p>
          {origin ? (
            <p className="text-body-sm text-tertiary">
              Nothing will be posted to #{origin.name} unless you choose to
              share it.
            </p>
          ) : null}
        </div>
        {pending ? (
          <div className="bootstrap-status" role="status">
            <span>
              {pending.pending === "failed"
                ? "Could not start the Session"
                : "Creating Session"}
            </span>
            <span className="text-body-sm text-tertiary">
              {pending.content}
            </span>
            {error ? (
              <span className="text-body-sm text-red-12">{error}</span>
            ) : null}
          </div>
        ) : null}
      </div>
      <div className="composer-dock">
        <MessageComposer
          draft={pending?.pending === "failed" ? pending.content : draft}
          disabled={Boolean(pending && pending.pending !== "failed")}
          onDraftChange={onDraftChange}
          onSend={onCreate}
          placeholder="Start a new session"
        />
      </div>
    </main>
  );
}
