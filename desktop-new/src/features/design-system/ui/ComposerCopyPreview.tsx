import { useState } from "react";
import { MessageComposer } from "@/features/composer/ui/MessageComposer";
import { Select } from "@/shared/ui/Select";

// Exact copy from Figma uhFH3LPsy6HMqzoATFgWKl, frame 809:12505.
const CONTEXTS = [
  {
    group: "Channel",
    value: "channel-start",
    label: "Channel · New message",
    copy: "Send a message in #buzz-design",
  },
  {
    group: "Channel",
    value: "channel-reply",
    label: "Channel · Reply",
    copy: "Reply in #buzz-design",
  },
  {
    group: "Session",
    value: "session-start",
    label: "Session · New session",
    copy: "Start a new session",
  },
  {
    group: "Session",
    value: "session-reply",
    label: "Session · Reply",
    copy: "Reply in this session",
  },
  {
    group: "Direct message",
    value: "dm-start",
    label: "Direct message · New message",
    copy: "Start a new direct message",
  },
  {
    group: "Direct message",
    value: "dm-reply",
    label: "Direct message · Reply",
    copy: "Reply in this direct message",
  },
] as const;
const GROUPS = ["Channel", "Session", "Direct message"].map((label) => ({
  label,
  options: CONTEXTS.filter((context) => context.group === label),
}));

/** Documentation-only context selection; real destinations own product copy. */
export function ComposerCopyPreview() {
  const [value, setValue] = useState<string>("channel-start");
  const [draft, setDraft] = useState("");
  const context =
    CONTEXTS.find((candidate) => candidate.value === value) ?? CONTEXTS[0];
  return (
    <>
      <Select
        label="Preview context"
        value={value}
        groups={GROUPS}
        onValueChange={setValue}
      />
      <div className="composer-playground-stage">
        <MessageComposer
          autoFocus={false}
          draft={draft}
          onDraftChange={setDraft}
          onSend={async () => undefined}
          placeholder={context.copy}
        />
      </div>
    </>
  );
}
