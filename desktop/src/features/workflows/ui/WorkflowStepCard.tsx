import { Trash2 } from "lucide-react";

import { cn } from "@/shared/lib/cn";
import { Button } from "@/shared/ui/button";
import { Input } from "@/shared/ui/input";
import { Textarea } from "@/shared/ui/textarea";
import { FieldLabel, FormSelect } from "./workflowFormPrimitives";
import { WorkflowWebhookHeadersEditor } from "./WorkflowWebhookHeadersEditor";
import type { StepFormState, TriggerType } from "./workflowFormTypes";

function BackendSupportHint({ action }: { action: StepFormState["action"] }) {
  switch (action) {
    case "send_dm":
      return (
        <p className="rounded-md border border-amber-500/30 bg-amber-500/10 px-2 py-1 text-xs text-amber-700">
          Backend note: `send_dm` is not executed yet, so runs fail at this
          step.
        </p>
      );
    case "set_channel_topic":
      return (
        <p className="rounded-md border border-amber-500/30 bg-amber-500/10 px-2 py-1 text-xs text-amber-700">
          Backend note: `set_channel_topic` is not executed yet, so runs fail at
          this step.
        </p>
      );
    case "request_approval":
      return (
        <p className="rounded-md border border-amber-500/30 bg-amber-500/10 px-2 py-1 text-xs text-amber-700">
          Backend note: approval gates still stop runs with WF-08; approval
          records are not persisted yet.
        </p>
      );
    default:
      return null;
  }
}

function StepConfigFields({
  step,
  prefix,
  disabled,
  triggerType,
  onUpdate,
}: {
  step: StepFormState;
  prefix: string;
  disabled?: boolean;
  triggerType: TriggerType;
  onUpdate: (step: StepFormState) => void;
}) {
  switch (step.action) {
    case "delay":
      return (
        <div className="space-y-1.5">
          <FieldLabel htmlFor={`${prefix}-duration`}>Duration</FieldLabel>
          <Input
            autoCapitalize="off"
            disabled={disabled}
            id={`${prefix}-duration`}
            onChange={(event) =>
              onUpdate({ ...step, duration: event.target.value })
            }
            placeholder="e.g. 5s, 1m, 1h"
            value={step.duration ?? ""}
          />
        </div>
      );
    case "send_message":
      return (
        <div className="space-y-2">
          <div className="space-y-1.5">
            <FieldLabel htmlFor={`${prefix}-text`}>Message text</FieldLabel>
            <Textarea
              autoCapitalize="off"
              className="min-h-[60px] resize-y text-xs"
              disabled={disabled}
              id={`${prefix}-text`}
              onChange={(event) =>
                onUpdate({ ...step, text: event.target.value })
              }
              placeholder="e.g. Deployment started by {{trigger.author}}"
              value={step.text ?? ""}
            />
          </div>
          <div className="space-y-1.5">
            <FieldLabel htmlFor={`${prefix}-channel`}>
              Channel override (optional)
            </FieldLabel>
            <Input
              autoCapitalize="off"
              disabled={disabled}
              id={`${prefix}-channel`}
              onChange={(event) =>
                onUpdate({ ...step, channel: event.target.value })
              }
              placeholder="Channel UUID"
              value={step.channel ?? ""}
            />
            <p className="text-xs text-muted-foreground">
              Defaults to the trigger channel. Webhook and manual triggers
              require a channel.
            </p>
            {triggerType === "webhook" && !(step.channel ?? "").trim() ? (
              <p className="text-xs text-amber-700">
                This step will fail for webhook-triggered runs until a channel
                override is set.
              </p>
            ) : null}
          </div>
        </div>
      );
    case "send_dm":
      return (
        <div className="space-y-2">
          <BackendSupportHint action={step.action} />
          <div className="space-y-1.5">
            <FieldLabel htmlFor={`${prefix}-to`}>To (pubkey)</FieldLabel>
            <Input
              autoCapitalize="off"
              disabled={disabled}
              id={`${prefix}-to`}
              onChange={(event) =>
                onUpdate({ ...step, to: event.target.value })
              }
              placeholder="e.g. {{trigger.author}} or hex pubkey"
              value={step.to ?? ""}
            />
          </div>
          <div className="space-y-1.5">
            <FieldLabel htmlFor={`${prefix}-text`}>Message text</FieldLabel>
            <Textarea
              autoCapitalize="off"
              className="min-h-[60px] resize-y text-xs"
              disabled={disabled}
              id={`${prefix}-text`}
              onChange={(event) =>
                onUpdate({ ...step, text: event.target.value })
              }
              placeholder="DM content"
              value={step.text ?? ""}
            />
          </div>
        </div>
      );
    case "call_webhook":
      return (
        <div className="space-y-4">
          <div className="space-y-1.5">
            <FieldLabel htmlFor={`${prefix}-url`}>Endpoint URL</FieldLabel>
            <Input
              autoCapitalize="off"
              disabled={disabled}
              id={`${prefix}-url`}
              onChange={(event) =>
                onUpdate({ ...step, url: event.target.value })
              }
              placeholder="https://..."
              value={step.url ?? ""}
            />
            {step.url && !step.url.startsWith("https://") ? (
              <p className="text-xs text-destructive">
                URL must start with https://
              </p>
            ) : null}
          </div>
          <div className="space-y-1.5">
            <FieldLabel htmlFor={`${prefix}-method`}>HTTP method</FieldLabel>
            <FormSelect
              disabled={disabled}
              id={`${prefix}-method`}
              onChange={(value) => onUpdate({ ...step, method: value })}
              value={step.method ?? "POST"}
            >
              <option value="POST">POST</option>
              <option value="GET">GET</option>
              <option value="PUT">PUT</option>
              <option value="PATCH">PATCH</option>
              <option value="DELETE">DELETE</option>
            </FormSelect>
          </div>
          <WorkflowWebhookHeadersEditor
            disabled={disabled}
            headers={step.headers ?? []}
            onChange={(headers) => onUpdate({ ...step, headers })}
            stepId={step.id || prefix}
          />
          <div className="space-y-1.5">
            <FieldLabel htmlFor={`${prefix}-body`}>
              Request body (optional)
            </FieldLabel>
            <Textarea
              autoCapitalize="off"
              className="min-h-[60px] resize-y font-mono text-xs"
              disabled={disabled}
              id={`${prefix}-body`}
              onChange={(event) =>
                onUpdate({ ...step, body: event.target.value })
              }
              placeholder='{"key": "{{trigger.text}}"}'
              value={step.body ?? ""}
            />
          </div>
        </div>
      );
    case "request_approval":
      return (
        <div className="space-y-2">
          <BackendSupportHint action={step.action} />
          <div className="space-y-1.5">
            <FieldLabel htmlFor={`${prefix}-from`}>From (approver)</FieldLabel>
            <Input
              autoCapitalize="off"
              disabled={disabled}
              id={`${prefix}-from`}
              onChange={(event) =>
                onUpdate({ ...step, from: event.target.value })
              }
              placeholder="Pubkey or role"
              value={step.from ?? ""}
            />
          </div>
          <div className="space-y-1.5">
            <FieldLabel htmlFor={`${prefix}-message`}>Message</FieldLabel>
            <Input
              autoCapitalize="off"
              disabled={disabled}
              id={`${prefix}-message`}
              onChange={(event) =>
                onUpdate({ ...step, message: event.target.value })
              }
              placeholder="Approval request message"
              value={step.message ?? ""}
            />
          </div>
          <div className="space-y-1.5">
            <FieldLabel htmlFor={`${prefix}-timeout`}>
              Timeout (optional)
            </FieldLabel>
            <Input
              autoCapitalize="off"
              disabled={disabled}
              id={`${prefix}-timeout`}
              onChange={(event) =>
                onUpdate({ ...step, timeout: event.target.value })
              }
              placeholder="e.g. 24h"
              value={step.timeout ?? ""}
            />
          </div>
        </div>
      );
    case "add_reaction":
      return (
        <div className="space-y-1.5">
          <FieldLabel htmlFor={`${prefix}-emoji`}>Emoji</FieldLabel>
          <Input
            autoCapitalize="off"
            disabled={disabled}
            id={`${prefix}-emoji`}
            onChange={(event) =>
              onUpdate({ ...step, emoji: event.target.value })
            }
            placeholder="e.g. thumbsup"
            value={step.emoji ?? ""}
          />
        </div>
      );
    case "set_channel_topic":
      return (
        <div className="space-y-2">
          <BackendSupportHint action={step.action} />
          <div className="space-y-1.5">
            <FieldLabel htmlFor={`${prefix}-topic`}>Topic</FieldLabel>
            <Input
              autoCapitalize="off"
              disabled={disabled}
              id={`${prefix}-topic`}
              onChange={(event) =>
                onUpdate({ ...step, topic: event.target.value })
              }
              placeholder="New channel topic"
              value={step.topic ?? ""}
            />
          </div>
        </div>
      );
    default:
      return null;
  }
}

function SectionHeading({ title }: { title: string }) {
  return <h4 className="text-sm font-semibold text-foreground">{title}</h4>;
}

export function WorkflowStepCard({
  bare = false,
  showHeader = true,
  index,
  disabled,
  onRemove,
  onUpdate,
  step,
  triggerType,
}: {
  bare?: boolean;
  showHeader?: boolean;
  index: number;
  disabled?: boolean;
  onRemove: () => void;
  onUpdate: (step: StepFormState) => void;
  step: StepFormState;
  triggerType: TriggerType;
}) {
  const prefix = `wf-step-${index}`;

  return (
    <div
      className={cn(
        "space-y-0",
        !bare && "rounded-lg border border-border/70 bg-muted/10 p-3",
      )}
    >
      {showHeader ? (
        <div className="mb-3 flex items-center justify-between gap-2">
          <span className="text-xs font-medium text-muted-foreground">
            Step {index + 1}
          </span>
          <Button
            aria-label="Remove step"
            className="h-7 w-7"
            disabled={disabled}
            onClick={onRemove}
            size="icon"
            type="button"
            variant="ghost"
          >
            <Trash2 className="h-4 w-4 text-muted-foreground" />
          </Button>
        </div>
      ) : null}

      <section className="space-y-4 pb-5">
        <StepConfigFields
          disabled={disabled}
          onUpdate={onUpdate}
          prefix={prefix}
          step={step}
          triggerType={triggerType}
        />
      </section>

      <section className="space-y-4 border-t border-border/50 py-5">
        <SectionHeading title="Run controls" />
        <div className="space-y-1.5">
          <FieldLabel htmlFor={`${prefix}-condition`}>
            Condition (optional)
          </FieldLabel>
          <Input
            autoCapitalize="off"
            disabled={disabled}
            id={`${prefix}-condition`}
            onChange={(event) =>
              onUpdate({ ...step, condition: event.target.value })
            }
            placeholder='e.g. str_contains(trigger_text, "deploy")'
            value={step.condition ?? ""}
          />
        </div>
        <div className="space-y-1.5">
          <FieldLabel htmlFor={`${prefix}-timeout-secs`}>
            Timeout (seconds)
          </FieldLabel>
          <Input
            autoCapitalize="off"
            disabled={disabled}
            id={`${prefix}-timeout-secs`}
            inputMode="numeric"
            onChange={(event) =>
              onUpdate({ ...step, timeoutSecs: event.target.value })
            }
            placeholder="e.g. 300"
            value={step.timeoutSecs ?? ""}
          />
          <p className="text-xs text-muted-foreground">
            Defaults to the workflow timeout.
          </p>
        </div>
      </section>

      <section className="space-y-4 border-t border-border/50 pt-5">
        <SectionHeading title="Step details" />
        <div className="space-y-1.5">
          <FieldLabel htmlFor={`${prefix}-name`}>Name (optional)</FieldLabel>
          <Input
            autoCapitalize="off"
            disabled={disabled}
            id={`${prefix}-name`}
            onChange={(event) =>
              onUpdate({ ...step, name: event.target.value })
            }
            placeholder="e.g. Notify deployment channel"
            value={step.name ?? ""}
          />
        </div>
        <div className="space-y-1.5">
          <FieldLabel htmlFor={`${prefix}-id`}>Step ID</FieldLabel>
          <Input
            autoCapitalize="off"
            disabled={disabled}
            id={`${prefix}-id`}
            onChange={(event) => onUpdate({ ...step, id: event.target.value })}
            placeholder="unique_step_id"
            value={step.id}
          />
          <p className="text-xs text-muted-foreground">
            Used in configuration and run history.
          </p>
        </div>
      </section>
    </div>
  );
}
