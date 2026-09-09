import { useRef, useState } from "react";
import { AIComposer, Button, IconButton, Select } from "@buzz/ui";
import {
  Check,
  ChevronDown,
  FileText,
  MessageSquare,
  ShieldCheck,
  X,
} from "lucide-react";

function ComposerChoice({
  label,
  value,
  options,
  disabled,
  onChange,
}: {
  label: string;
  value: string;
  options: readonly { value: string; label: string }[];
  disabled: boolean;
  onChange: (value: string) => void;
}) {
  return (
    <Select.Root
      value={value}
      onValueChange={(next) => {
        if (next) onChange(next);
      }}
      items={options}
      disabled={disabled}
    >
      <Select.Trigger aria-label={label}>
        {label === "Action policy" && (
          <ShieldCheck aria-hidden="true" className="bui-icon" />
        )}
        <Select.Value />
        <Select.Icon>
          <ChevronDown aria-hidden="true" className="bui-icon" />
        </Select.Icon>
      </Select.Trigger>
      <Select.Portal>
        <Select.Positioner sideOffset={8}>
          <Select.Popup>
            <Select.List>
              {options.map((option) => (
                <Select.Item key={option.value} value={option.value}>
                  <Select.ItemText>{option.label}</Select.ItemText>
                  <Select.ItemIndicator>
                    <Check aria-hidden="true" />
                  </Select.ItemIndicator>
                </Select.Item>
              ))}
            </Select.List>
          </Select.Popup>
        </Select.Positioner>
      </Select.Portal>
    </Select.Root>
  );
}

export function ComposerDemo() {
  const [value, setValue] = useState("");
  const [annotation, setAnnotation] = useState(true);
  const [files, setFiles] = useState<Array<{ id: string; file: File }>>([]);
  const [model, setModel] = useState("balanced");
  const [policy, setPolicy] = useState("ask");
  const [running, setRunning] = useState(false);
  const [failNext, setFailNext] = useState(false);
  const [status, setStatus] = useState(
    "Local preview. No messages, files, or audio leave this page.",
  );
  const [attachmentError, setAttachmentError] = useState("");
  const fileInput = useRef<HTMLInputElement>(null);
  return (
    <div className="bui-stack">
      <input
        ref={fileInput}
        type="file"
        multiple
        hidden
        aria-label="Choose composer attachments"
        disabled={running}
        onChange={(event) => {
          const incoming = Array.from(event.currentTarget.files ?? []);
          event.currentTarget.value = "";
          if (
            files.length + incoming.length > 5 ||
            incoming.some((file) => file.size > 10 * 1024 * 1024)
          ) {
            setAttachmentError(
              "Choose up to 5 files, each no larger than 10 MB.",
            );
            return;
          }
          setAttachmentError("");
          setFiles((current) => [
            ...current,
            ...incoming.map((file) => ({ id: crypto.randomUUID(), file })),
          ]);
        }}
      />
      <AIComposer
        label="Prompt the assistant"
        placeholder="What would you like to build?"
        value={value}
        onValueChange={setValue}
        onAttach={() => fileInput.current?.click()}
        voice={{
          label: "Insert demo voice transcript",
          onToggle: () => {
            setValue("Help me refine this design system.");
            setStatus(
              "Sample transcript inserted. The microphone was not accessed.",
            );
          },
        }}
        generation={
          running
            ? {
                onStop: () => {
                  setRunning(false);
                  setStatus("Preview stopped. You can write another prompt.");
                },
              }
            : undefined
        }
        onSubmit={(text) => {
          if (failNext) {
            setFailNext(false);
            throw new Error("Simulated submission failure");
          }
          setStatus(
            `Preview accepted: “${text}” · ${model} · ${policy} · ${files.length} attachments · ${annotation ? 1 : 0} annotations. No request was sent.`,
          );
          setValue("");
          setFiles([]);
          setRunning(true);
        }}
        context={({ disabled }) => (
          <>
            {annotation && (
              <span className="bui-chip">
                <MessageSquare aria-hidden="true" className="bui-icon" />1
                annotation
                <IconButton
                  aria-label="Remove annotation"
                  variant="ghost"
                  size="sm"
                  disabled={disabled}
                  onClick={() => setAnnotation(false)}
                >
                  <X aria-hidden="true" />
                </IconButton>
              </span>
            )}
            {files.map(({ id, file }) => (
              <span key={id} className="bui-chip">
                <FileText aria-hidden="true" className="bui-icon" />
                <span className="min-w-0 wrap-anywhere">{file.name}</span>
                <IconButton
                  aria-label={`Remove attachment ${file.name}`}
                  variant="ghost"
                  size="sm"
                  disabled={disabled}
                  onClick={() =>
                    setFiles((current) =>
                      current.filter((item) => item.id !== id),
                    )
                  }
                >
                  <X aria-hidden="true" />
                </IconButton>
              </span>
            ))}
          </>
        )}
        controls={({ disabled }) => (
          <>
            <ComposerChoice
              label="Model"
              value={model}
              onChange={setModel}
              disabled={disabled}
              options={[
                { value: "balanced", label: "Balanced" },
                { value: "fast", label: "Fast" },
              ]}
            />
            <ComposerChoice
              label="Action policy"
              value={policy}
              onChange={setPolicy}
              disabled={disabled}
              options={[
                { value: "ask", label: "Ask first" },
                { value: "read", label: "Read only" },
              ]}
            />
          </>
        )}
      />
      {attachmentError && (
        <p className="bui-field-error" role="alert">
          {attachmentError}
        </p>
      )}
      <p className="text-caption text-secondary wrap-anywhere" role="status">
        {status}
      </p>
      <div className="bui-inline">
        {running ? (
          <Button
            variant="outline"
            size="sm"
            onClick={() => {
              setRunning(false);
              setStatus("Preview complete. Ready for another prompt.");
            }}
          >
            Finish preview
          </Button>
        ) : (
          <Button
            variant="ghost"
            size="sm"
            aria-pressed={failNext}
            onClick={() => setFailNext(!failNext)}
          >
            Simulate send failure
          </Button>
        )}
        {!annotation && (
          <Button
            size="sm"
            variant="ghost"
            disabled={running}
            onClick={() => setAnnotation(true)}
          >
            Restore annotation
          </Button>
        )}
      </div>
    </div>
  );
}
