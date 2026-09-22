import { ChevronRight, Trash2 } from "lucide-react";
import { type ReactNode, useState } from "react";

import { i18n, useTranslation } from "@/i18n";
import { cn } from "@/shared/lib/cn";
import { Button } from "@/shared/ui/button";
import { Checkbox } from "@/shared/ui/checkbox";
import { Input } from "@/shared/ui/input";
import { Textarea } from "@/shared/ui/textarea";
import { WorkflowTemplateTextarea } from "./WorkflowTemplateTextarea";
import { WorkflowDurationField } from "./WorkflowDurationField";
import { WorkflowMessageTextCondition } from "./WorkflowMessageTextConditionEditor";
import { FieldLabel, FormSelect } from "./workflowFormPrimitives";
import { WorkflowWebhookHeadersEditor } from "./WorkflowWebhookHeadersEditor";
import {
  isThreadReplyEligibleTrigger,
  supportsMessageTextCondition,
  type StepFormState,
  type TriggerType,
} from "./workflowFormTypes";

const DEFAULT_STEP_TIMEOUT_SECONDS = 5 * 60;

type StepSetting = "run-controls" | "details";

function StepSettingAccordion({
  children,
  disabled,
  expanded,
  label,
  onToggle,
  summary,
}: {
  children: ReactNode;
  disabled?: boolean;
  expanded: boolean;
  label: string;
  onToggle: () => void;
  summary: string;
}) {
  return (
    <div>
      <button
        aria-expanded={expanded}
        className="flex min-h-12 w-full items-center gap-3 py-3 text-left transition-colors hover:text-foreground focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-inset focus-visible:ring-ring disabled:cursor-not-allowed disabled:opacity-50"
        disabled={disabled}
        onClick={onToggle}
        type="button"
      >
        <span className="min-w-0 flex-1 truncate text-base font-medium">
          {label}
        </span>
        <span className="max-w-40 truncate text-sm text-muted-foreground">
          {summary}
        </span>
        <ChevronRight
          className={cn(
            "h-4 w-4 shrink-0 text-muted-foreground/70 transition-transform duration-150 motion-reduce:transition-none",
            expanded && "rotate-90",
          )}
        />
      </button>

      {expanded ? (
        <div className="animate-in pb-4 pt-1 fade-in slide-in-from-top-1 duration-150 motion-reduce:animate-none">
          {children}
        </div>
      ) : null}
    </div>
  );
}

function runControlsSummary(step: StepFormState): string {
  const hasCondition = Boolean(step.condition?.trim());
  const timeout = step.timeoutSecs?.trim();
  if (hasCondition && timeout)
    return i18n.t("workflows.step-card.run-controls-conditional-timeout", {
      timeout,
    });
  if (hasCondition)
    return i18n.t("workflows.step-card.run-controls-conditional");
  if (timeout) return timeout;
  return i18n.t("workflows.step-card.run-controls-default");
}

function BackendSupportHint({ action }: { action: StepFormState["action"] }) {
  const { t } = useTranslation();
  switch (action) {
    case "send_dm":
      return (
        <p className="rounded-md border border-amber-500/30 bg-amber-500/10 px-2 py-1 text-xs text-amber-700">
          {t("workflows.step-card.backend-note-send-dm")}
        </p>
      );
    case "set_channel_topic":
      return (
        <p className="rounded-md border border-amber-500/30 bg-amber-500/10 px-2 py-1 text-xs text-amber-700">
          {t("workflows.step-card.backend-note-set-channel-topic")}
        </p>
      );
    case "request_approval":
      return (
        <p className="rounded-md border border-amber-500/30 bg-amber-500/10 px-2 py-1 text-xs text-amber-700">
          {t("workflows.step-card.backend-note-request-approval")}
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
  previousSteps,
  triggerType,
  workflowChannelId,
  onUpdate,
}: {
  step: StepFormState;
  prefix: string;
  disabled?: boolean;
  previousSteps: StepFormState[];
  triggerType: TriggerType;
  workflowChannelId?: string | null;
  onUpdate: (step: StepFormState) => void;
}) {
  const { t } = useTranslation();
  switch (step.action) {
    case "delay":
      return (
        <div className="space-y-1.5">
          <FieldLabel htmlFor={`${prefix}-duration`}>
            {t("workflows.step.duration")}
          </FieldLabel>
          <Input
            autoCapitalize="off"
            disabled={disabled}
            id={`${prefix}-duration`}
            onChange={(event) =>
              onUpdate({ ...step, duration: event.target.value })
            }
            placeholder={t("workflows.step-card.duration-placeholder")}
            value={step.duration ?? ""}
          />
        </div>
      );
    case "send_message":
      return (
        <div className="space-y-2">
          <div className="space-y-1.5">
            <FieldLabel htmlFor={`${prefix}-text`}>
              {t("workflows.step-card.message-text")}
            </FieldLabel>
            <WorkflowTemplateTextarea
              autoCapitalize="off"
              className="min-h-[60px] resize-y text-xs"
              disabled={disabled}
              id={`${prefix}-text`}
              onValueChange={(text) => onUpdate({ ...step, text })}
              placeholder={t("workflows.step-card.message-text-placeholder")}
              previousSteps={previousSteps}
              triggerType={triggerType}
              value={step.text ?? ""}
            />
          </div>
          {workflowChannelId ? (
            <p className="text-xs text-muted-foreground">
              {t("workflows.step-card.channel-hint")}
            </p>
          ) : (
            <div className="space-y-1.5">
              <FieldLabel htmlFor={`${prefix}-channel`}>
                {t("workflows.step-card.channel-override")}
              </FieldLabel>
              <Input
                autoCapitalize="off"
                disabled={disabled}
                id={`${prefix}-channel`}
                onChange={(event) =>
                  onUpdate({ ...step, channel: event.target.value })
                }
                placeholder={t("workflows.template.channel-uuid")}
                value={step.channel ?? ""}
              />
              <p className="text-xs text-muted-foreground">
                {t("workflows.step-card.channel-defaults")}
              </p>
              {triggerType === "webhook" && !(step.channel ?? "").trim() ? (
                <p className="text-xs text-amber-700">
                  {t("workflows.step-card.webhook-channel-warning")}
                </p>
              ) : null}
            </div>
          )}
          {isThreadReplyEligibleTrigger(triggerType) ? (
            <div className="flex items-center gap-2">
              <Checkbox
                checked={step.replyInThread === true}
                disabled={disabled}
                id={`${prefix}-reply-in-thread`}
                onCheckedChange={(checked) =>
                  onUpdate({ ...step, replyInThread: checked === true })
                }
              />
              <label className="text-xs" htmlFor={`${prefix}-reply-in-thread`}>
                {t("workflows.step-card.reply-in-thread")}
              </label>
            </div>
          ) : null}
        </div>
      );
    case "send_dm":
      return (
        <div className="space-y-2">
          <BackendSupportHint action={step.action} />
          <div className="space-y-1.5">
            <FieldLabel htmlFor={`${prefix}-to`}>
              {t("workflows.step-card.to-pubkey")}
            </FieldLabel>
            <Input
              autoCapitalize="off"
              disabled={disabled}
              id={`${prefix}-to`}
              onChange={(event) =>
                onUpdate({ ...step, to: event.target.value })
              }
              placeholder={t("workflows.step-card.to-pubkey-placeholder")}
              value={step.to ?? ""}
            />
          </div>
          <div className="space-y-1.5">
            <FieldLabel htmlFor={`${prefix}-text`}>
              {t("workflows.step-card.message-text")}
            </FieldLabel>
            <Textarea
              autoCapitalize="off"
              className="min-h-[60px] resize-y text-xs"
              disabled={disabled}
              id={`${prefix}-text`}
              onChange={(event) =>
                onUpdate({ ...step, text: event.target.value })
              }
              placeholder={t("workflows.step-card.dm-content-placeholder")}
              value={step.text ?? ""}
            />
          </div>
        </div>
      );
    case "call_webhook":
      return (
        <div className="space-y-4">
          <div className="space-y-1.5">
            <FieldLabel htmlFor={`${prefix}-url`}>
              {t("workflows.step-card.endpoint-url")}
            </FieldLabel>
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
                {t("workflows.step-card.url-https-error")}
              </p>
            ) : null}
          </div>
          <div className="space-y-1.5">
            <FieldLabel htmlFor={`${prefix}-method`}>
              {t("workflows.step-card.method")}
            </FieldLabel>
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
              {t("workflows.step-card.request-body")}
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
            <FieldLabel htmlFor={`${prefix}-from`}>
              {t("workflows.step-card.from-approver")}
            </FieldLabel>
            <Input
              autoCapitalize="off"
              disabled={disabled}
              id={`${prefix}-from`}
              onChange={(event) =>
                onUpdate({ ...step, from: event.target.value })
              }
              placeholder={t("workflows.step-card.from-role-placeholder")}
              value={step.from ?? ""}
            />
          </div>
          <div className="space-y-1.5">
            <FieldLabel htmlFor={`${prefix}-message`}>
              {t("workflows.step-card.message")}
            </FieldLabel>
            <Input
              autoCapitalize="off"
              disabled={disabled}
              id={`${prefix}-message`}
              onChange={(event) =>
                onUpdate({ ...step, message: event.target.value })
              }
              placeholder={t(
                "workflows.step-card.approval-message-placeholder",
              )}
              value={step.message ?? ""}
            />
          </div>
          <div className="space-y-1.5">
            <FieldLabel htmlFor={`${prefix}-timeout`}>
              {t("workflows.step-card.timeout-optional")}
            </FieldLabel>
            <Input
              autoCapitalize="off"
              disabled={disabled}
              id={`${prefix}-timeout`}
              onChange={(event) =>
                onUpdate({ ...step, timeout: event.target.value })
              }
              placeholder={t("workflows.step-card.timeout-hours-placeholder")}
              value={step.timeout ?? ""}
            />
          </div>
        </div>
      );
    case "add_reaction":
      return (
        <div className="space-y-1.5">
          <FieldLabel htmlFor={`${prefix}-emoji`}>
            {t("workflows.step-card.emoji")}
          </FieldLabel>
          <Input
            autoCapitalize="off"
            disabled={disabled}
            id={`${prefix}-emoji`}
            onChange={(event) =>
              onUpdate({ ...step, emoji: event.target.value })
            }
            placeholder={t("workflows.step-card.emoji-name-placeholder")}
            value={step.emoji ?? ""}
          />
        </div>
      );
    case "set_channel_topic":
      return (
        <div className="space-y-2">
          <BackendSupportHint action={step.action} />
          <div className="space-y-1.5">
            <FieldLabel htmlFor={`${prefix}-topic`}>
              {t("workflows.step-card.topic")}
            </FieldLabel>
            <Input
              autoCapitalize="off"
              disabled={disabled}
              id={`${prefix}-topic`}
              onChange={(event) =>
                onUpdate({ ...step, topic: event.target.value })
              }
              placeholder={t("workflows.step-card.topic-placeholder")}
              value={step.topic ?? ""}
            />
          </div>
        </div>
      );
    default:
      return null;
  }
}

export function WorkflowStepCard({
  bare = false,
  showHeader = true,
  index,
  disabled,
  onRemove,
  onUpdate,
  step,
  previousSteps = [],
  triggerType,
  workflowChannelId,
}: {
  bare?: boolean;
  showHeader?: boolean;
  index: number;
  disabled?: boolean;
  onRemove: () => void;
  onUpdate: (step: StepFormState) => void;
  step: StepFormState;
  previousSteps?: StepFormState[];
  triggerType: TriggerType;
  workflowChannelId?: string | null;
}) {
  const { t } = useTranslation();
  const prefix = `wf-step-${index}`;
  const [expandedSetting, setExpandedSetting] = useState<StepSetting | null>(
    null,
  );

  const toggleSetting = (setting: StepSetting) => {
    setExpandedSetting((current) => (current === setting ? null : setting));
  };

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
            {t("workflows.form.step-title", { number: index + 1 })}
          </span>
          <Button
            aria-label={t("workflows.form.remove-step")}
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
          previousSteps={previousSteps}
          step={step}
          triggerType={triggerType}
          workflowChannelId={workflowChannelId}
        />
      </section>

      <section className="divide-y divide-border/50 border-t border-border/50">
        <StepSettingAccordion
          disabled={disabled}
          expanded={expandedSetting === "run-controls"}
          label={t("workflows.step-card.run-controls")}
          onToggle={() => toggleSetting("run-controls")}
          summary={runControlsSummary(step)}
        >
          <div className="space-y-4">
            {supportsMessageTextCondition(triggerType) ? (
              <WorkflowMessageTextCondition
                allowAdvanced={false}
                disabled={disabled}
                onChange={(condition) => onUpdate({ ...step, condition })}
                value={step.condition ?? ""}
              />
            ) : null}
            <WorkflowDurationField
              disabled={disabled}
              fallbackSeconds={DEFAULT_STEP_TIMEOUT_SECONDS}
              id={`${prefix}-timeout-secs`}
              label={t("workflows.step-card.timeout")}
              onChange={(timeoutSecs) => onUpdate({ ...step, timeoutSecs })}
              placeholder="5m"
              value={step.timeoutSecs ?? ""}
            />
          </div>
        </StepSettingAccordion>

        <StepSettingAccordion
          disabled={disabled}
          expanded={expandedSetting === "details"}
          label={t("workflows.step-card.details")}
          onToggle={() => toggleSetting("details")}
          summary={step.name?.trim() || step.id}
        >
          <div className="space-y-4">
            <div className="space-y-1.5">
              <FieldLabel htmlFor={`${prefix}-name`}>
                {t("workflows.step-card.name")}
              </FieldLabel>
              <Input
                autoCapitalize="off"
                disabled={disabled}
                id={`${prefix}-name`}
                onChange={(event) =>
                  onUpdate({ ...step, name: event.target.value })
                }
                placeholder={t("workflows.step-card.name-placeholder")}
                value={step.name ?? ""}
              />
            </div>
            <div className="space-y-1.5">
              <FieldLabel htmlFor={`${prefix}-id`}>
                {t("workflows.step-card.step-id")}
              </FieldLabel>
              <Input
                autoCapitalize="off"
                disabled={disabled}
                id={`${prefix}-id`}
                onChange={(event) =>
                  onUpdate({ ...step, id: event.target.value })
                }
                placeholder="unique_step_id"
                value={step.id}
              />
              <p className="text-xs text-muted-foreground">
                {t("workflows.step-card.step-id-hint")}
              </p>
            </div>
          </div>
        </StepSettingAccordion>
      </section>
    </div>
  );
}
