import { type ReactNode, useEffect, useRef, useState } from "react";
import { MessageComposer } from "@/features/composer/ui/MessageComposer";
import { Button } from "@/shared/ui/Button";
import { ComposerCopyPreview } from "./ComposerCopyPreview";

function Example({
  title,
  description,
  children,
}: {
  title: string;
  description: string;
  children: ReactNode;
}) {
  return (
    <section className="component-specimen-group" aria-label={title}>
      <h3 className="text-heading text-primary">{title}</h3>
      <p className="text-body text-tertiary">{description}</p>
      {children}
    </section>
  );
}

function DraftExample({
  initial = "",
  formatting = false,
  disabled = false,
  responseControl,
}: {
  initial?: string;
  formatting?: boolean;
  disabled?: boolean;
  responseControl?: { state: "responding" | "stopping" };
}) {
  const [draft, setDraft] = useState(initial);
  return (
    <MessageComposer
      autoFocus={false}
      defaultFormattingOpen={formatting}
      disabled={disabled}
      draft={draft}
      onDraftChange={setDraft}
      onSend={async () => undefined}
      placeholder="Reply in this session"
      responseControl={responseControl}
    />
  );
}

/** Exercise the real submit path instead of painting a pretend sending/error state. */
function SubmissionExample() {
  const [draft, setDraft] = useState("Here is the updated design.");
  const [pending, setPending] = useState(false);
  const deferred = useRef<{
    resolve: () => void;
    reject: (error: Error) => void;
  } | null>(null);
  useEffect(
    () => () => {
      deferred.current?.resolve();
    },
    [],
  );
  function settle(fail: boolean) {
    if (fail) deferred.current?.reject(new Error("Example send failure"));
    else deferred.current?.resolve();
    deferred.current = null;
    setPending(false);
  }
  return (
    <>
      <MessageComposer
        autoFocus={false}
        draft={draft}
        onDraftChange={setDraft}
        onSend={() => {
          setPending(true);
          return new Promise<void>((resolve, reject) => {
            deferred.current = { resolve, reject };
          });
        }}
        placeholder="Reply in this session"
      />
      <div className="component-specimen-row">
        <Button
          size="compact"
          disabled={!pending}
          onClick={() => settle(false)}
        >
          Complete example send
        </Button>
        <Button size="compact" disabled={!pending} onClick={() => settle(true)}>
          Fail example send
        </Button>
      </div>
    </>
  );
}

/** One interactive playground, with only hard-to-reach states shown separately. */
export function ComposerStateGallery() {
  return (
    <div className="component-specimen-stack">
      <section
        className="component-specimen-group"
        aria-label="Composer playground"
      >
        <ComposerCopyPreview />
      </section>
      <h2 className="text-heading text-primary">States</h2>
      <Example
        title="Sending and failed"
        description="A message is being sent; if sending fails, the draft stays here so you can try again."
      >
        <SubmissionExample />
      </Example>
      <Example
        title="Unavailable"
        description="Writing and sending are disabled in this conversation."
      >
        <DraftExample disabled />
      </Example>
      <Example
        title="Responding"
        description="An agent is generating a response. Select Stop to cancel it."
      >
        <DraftExample responseControl={{ state: "responding" }} />
      </Example>
      <Example
        title="Stopping"
        description="Stop has been requested. The control is disabled while the agent finishes stopping."
      >
        <DraftExample responseControl={{ state: "stopping" }} />
      </Example>
    </div>
  );
}
