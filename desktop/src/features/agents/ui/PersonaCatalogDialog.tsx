import * as React from "react";

import { isCatalogPersonaSelected } from "@/features/agents/lib/catalog";
import { ProfileAvatar } from "@/features/profile/ui/ProfileAvatar";
import type { AgentPersona } from "@/shared/api/types";
import { useFeedbackToasts } from "@/shared/hooks/useToastEffect";
import { cn } from "@/shared/lib/cn";
import { Button } from "@/shared/ui/button";
import { Dialog } from "@/shared/ui/dialog";
import { ChooserDialogContent } from "@/shared/ui/chooser-dialog-content";
import { Markdown } from "@/shared/ui/markdown";
import { Skeleton } from "@/shared/ui/skeleton";

import { PersonaAddedBy } from "./PersonaAddedBy";
import { personaCatalogCopy } from "./personaLibraryCopy";

type PersonaCatalogDialogProps = {
  error: Error | null;
  feedbackErrorMessage: string | null;
  feedbackNoticeMessage: string | null;
  isLoading: boolean;
  isPending: boolean;
  onClearFeedback: () => void;
  onOpenChange: (open: boolean) => void;
  onSelectPersona: (persona: AgentPersona, active: boolean) => void;
  open: boolean;
  personas: AgentPersona[];
};

const agentInstructionMarkdownClassName = [
  "mt-3 leading-6 text-muted-foreground [&_blockquote]:!text-muted-foreground [&_code]:!text-muted-foreground [&_li]:text-muted-foreground [&_ol]:text-muted-foreground [&_p]:text-muted-foreground [&_strong]:text-muted-foreground [&_td]:text-muted-foreground [&_ul]:text-muted-foreground",
  "[&>h1]:!text-sm [&>h1]:!font-semibold [&>h1]:!leading-6 [&>h1]:!tracking-normal [&>h1]:!text-foreground",
  "[&>h2]:!text-sm [&>h2]:!font-semibold [&>h2]:!leading-6 [&>h2]:!tracking-normal [&>h2]:!text-foreground",
  "[&>h3]:!text-sm [&>h3]:!font-semibold [&>h3]:!leading-6 [&>h3]:!tracking-normal [&>h3]:!text-foreground",
  "[&>h4]:!text-sm [&>h4]:!font-semibold [&>h4]:!leading-6 [&>h4]:!tracking-normal [&>h4]:!text-foreground",
  "[&>h5]:!text-sm [&>h5]:!font-semibold [&>h5]:!leading-6 [&>h5]:!tracking-normal [&>h5]:!text-foreground",
  "[&>h6]:!text-sm [&>h6]:!font-semibold [&>h6]:!leading-6 [&>h6]:!tracking-normal [&>h6]:!text-foreground",
].join(" ");

export function PersonaCatalogDialog({
  error,
  feedbackErrorMessage,
  feedbackNoticeMessage,
  isLoading,
  isPending,
  onClearFeedback,
  onOpenChange,
  onSelectPersona,
  open,
  personas,
}: PersonaCatalogDialogProps) {
  const contentRef = React.useRef<HTMLDivElement | null>(null);
  const [selectedPersonaId, setSelectedPersonaId] = React.useState<
    string | null
  >(null);
  const selectedPersona = React.useMemo(() => {
    if (personas.length === 0) {
      return null;
    }

    return (
      personas.find((persona) => persona.id === selectedPersonaId) ??
      personas[0]
    );
  }, [personas, selectedPersonaId]);

  React.useEffect(() => {
    if (!open) {
      return;
    }

    if (personas.length === 0) {
      setSelectedPersonaId(null);
      return;
    }

    setSelectedPersonaId((current) =>
      current && personas.some((persona) => persona.id === current)
        ? current
        : personas[0].id,
    );
  }, [open, personas]);

  useFeedbackToasts(feedbackNoticeMessage, feedbackErrorMessage);

  const selectedPersonaIsActive = selectedPersona
    ? isCatalogPersonaSelected(selectedPersona)
    : false;

  const handleUseSelectedPersona = () => {
    if (!selectedPersona || selectedPersonaIsActive) {
      return;
    }

    onClearFeedback();
    onSelectPersona(selectedPersona, true);
  };

  return (
    <Dialog onOpenChange={onOpenChange} open={open}>
      <ChooserDialogContent
        className="h-[42rem] max-w-4xl"
        contentClassName="flex min-h-0 flex-1 p-0"
        data-testid="persona-catalog-dialog"
        description={personaCatalogCopy.dialogDescription}
        headerClassName="bg-sidebar pb-3 text-sidebar-foreground"
        headerTestId="persona-catalog-dialog-header"
        onOpenAutoFocus={(event) => {
          event.preventDefault();
          contentRef.current?.focus();
        }}
        ref={contentRef}
        scrollAreaClassName="flex min-h-0 overflow-hidden px-0"
        scrollAreaTestId="persona-catalog-dialog-body"
        tabIndex={-1}
        title={personaCatalogCopy.dialogTitle}
      >
        <PersonaCatalogChooser
          error={error}
          isLoading={isLoading}
          isPending={isPending}
          isSelectedPersonaActive={selectedPersonaIsActive}
          onUsePersona={handleUseSelectedPersona}
          onSelectPersona={setSelectedPersonaId}
          personas={personas}
          selectedPersona={selectedPersona}
          selectedPersonaId={selectedPersona?.id ?? null}
        />
      </ChooserDialogContent>
    </Dialog>
  );
}

type PersonaCatalogChooserProps = {
  error: Error | null;
  isLoading: boolean;
  isPending: boolean;
  isSelectedPersonaActive: boolean;
  onUsePersona: () => void;
  onSelectPersona: (personaId: string) => void;
  personas: AgentPersona[];
  selectedPersona: AgentPersona | null;
  selectedPersonaId: string | null;
};

function PersonaCatalogChooser({
  error,
  isLoading,
  isPending,
  isSelectedPersonaActive,
  onUsePersona,
  onSelectPersona,
  personas,
  selectedPersona,
  selectedPersonaId,
}: PersonaCatalogChooserProps) {
  return (
    <div className="flex min-h-0 flex-1 flex-col overflow-hidden bg-sidebar sm:flex-row">
      <div className="flex max-h-56 min-h-0 flex-col sm:max-h-none sm:w-56">
        <div
          className="min-h-0 flex-1 overflow-y-auto px-2 py-3"
          data-testid="persona-catalog-dialog-scroll-area"
        >
          {isLoading ? <PersonaCatalogListSkeleton /> : null}

          {!isLoading && personas.length > 0 ? (
            <div className="space-y-1">
              {personas.map((persona) => {
                const isCurrent = persona.id === selectedPersonaId;

                return (
                  <button
                    aria-current={isCurrent ? "true" : undefined}
                    className={cn(
                      "flex w-full items-center gap-2 rounded-lg px-4 py-1.5 text-left transition-[background-color,color,box-shadow] focus:outline-hidden focus-visible:ring-2 focus-visible:ring-sidebar-ring/50 focus-visible:ring-offset-2 focus-visible:ring-offset-sidebar",
                      isCurrent
                        ? "bg-sidebar-active text-sidebar-active-foreground"
                        : "text-sidebar-foreground/70 hover:bg-sidebar-accent hover:text-sidebar-accent-foreground",
                    )}
                    data-testid={`persona-catalog-list-item-${persona.id}`}
                    key={persona.id}
                    onClick={() => {
                      onSelectPersona(persona.id);
                    }}
                    type="button"
                  >
                    <ProfileAvatar
                      avatarUrl={persona.avatarUrl}
                      className="h-6 w-6 text-3xs"
                      label={persona.displayName}
                    />
                    <span className="min-w-0 flex-1 truncate text-sm font-medium">
                      {persona.displayName}
                    </span>
                  </button>
                );
              })}
            </div>
          ) : null}
        </div>
      </div>

      <div className="relative z-10 ml-px flex min-h-0 flex-1 flex-col overflow-hidden rounded-tl-xl bg-background shadow-[-1px_0_0_0_hsl(var(--sidebar-border)/0.45)]">
        <div
          className="min-h-0 flex-1 overflow-y-auto px-5 pb-24 pt-5"
          data-testid="persona-catalog-detail-pane"
        >
          {isLoading ? <PersonaCatalogDetailSkeleton /> : null}

          {!isLoading && selectedPersona ? (
            <PersonaCatalogDetail persona={selectedPersona} />
          ) : null}

          {!isLoading && personas.length === 0 && !error ? (
            <div className="flex min-h-80 items-center justify-center rounded-lg border border-dashed border-border/70 px-6 text-center">
              <div>
                <p className="text-sm font-semibold">
                  {personaCatalogCopy.emptyCatalogTitle}
                </p>
                <p className="mt-2 max-w-sm text-sm text-muted-foreground">
                  {personaCatalogCopy.emptyCatalogDescription}
                </p>
              </div>
            </div>
          ) : null}

          {error ? (
            <p className="rounded-lg border border-destructive/30 bg-destructive/10 px-4 py-3 text-sm text-destructive">
              {error.message}
            </p>
          ) : null}
        </div>

        <div className="pointer-events-none absolute inset-x-0 bottom-0 flex justify-end bg-gradient-to-t from-background via-background/95 to-transparent px-5 pb-4 pt-12">
          <Button
            aria-label={
              selectedPersona && isSelectedPersonaActive
                ? `${selectedPersona.displayName} is already in My Agents`
                : selectedPersona
                  ? `Add ${selectedPersona.displayName} from Agent Catalog`
                  : undefined
            }
            data-testid={
              selectedPersona
                ? `persona-catalog-use-agent-target-${selectedPersona.id}`
                : "persona-catalog-use-agent-target"
            }
            className="pointer-events-auto"
            disabled={!selectedPersona || isSelectedPersonaActive || isPending}
            onClick={onUsePersona}
            size="sm"
            type="button"
          >
            {isSelectedPersonaActive
              ? personaCatalogCopy.addedAction
              : personaCatalogCopy.useAction}
          </Button>
        </div>
      </div>
    </div>
  );
}

function PersonaCatalogDetail({ persona }: { persona: AgentPersona }) {
  return (
    <div className="space-y-6">
      <div className="flex items-center gap-3">
        <ProfileAvatar
          avatarUrl={persona.avatarUrl}
          className="h-12 w-12 text-sm"
          label={persona.displayName}
        />
        <div className="min-w-0">
          <h3 className="truncate text-xl font-semibold leading-snug">
            {persona.displayName}
          </h3>
          {persona.isBuiltIn ? null : <PersonaAddedBy className="mt-0.5" />}
        </div>
      </div>

      <PersonaCatalogMetaGroup
        items={[
          {
            label: "Type",
            value: persona.isBuiltIn ? "Built-in persona" : "Custom persona",
          },
          {
            label: "Preferred model",
            value: persona.model ?? "Use app default",
          },
          {
            label: "Preferred runtime",
            value: persona.runtime ?? "Use app default",
          },
        ]}
      />

      <div className="pt-3">
        <p className="text-base font-semibold text-foreground">
          Agent instruction
        </p>
        <Markdown
          className={agentInstructionMarkdownClassName}
          content={persona.systemPrompt}
          interactive={false}
        />
      </div>
    </div>
  );
}

function PersonaCatalogMetaGroup({
  items,
}: {
  items: { label: string; value: string }[];
}) {
  return (
    <div className="rounded-lg border border-border/70 bg-card/70">
      <div className="grid sm:grid-cols-3">
        {items.map((item, index) => (
          <div
            className={cn(
              "relative px-4 py-3",
              index > 0 &&
                "border-t border-border/60 sm:border-t-0 sm:before:absolute sm:before:bottom-3 sm:before:left-0 sm:before:top-3 sm:before:w-px sm:before:bg-border/70",
            )}
            key={item.label}
          >
            <p className="text-xs font-semibold text-muted-foreground">
              {item.label}
            </p>
            <p className="mt-2 text-sm font-medium text-foreground">
              {item.value}
            </p>
          </div>
        ))}
      </div>
    </div>
  );
}

function PersonaCatalogListSkeleton() {
  return (
    <div className="space-y-2">
      {["first", "second", "third", "fourth", "fifth"].map((key) => (
        <div
          className="flex items-center gap-2 rounded-lg px-4 py-1.5"
          key={key}
        >
          <Skeleton className="h-6 w-6 rounded-full" />
          <Skeleton className="h-4 w-28" />
        </div>
      ))}
    </div>
  );
}

function PersonaCatalogDetailSkeleton() {
  return (
    <div className="space-y-6">
      <div className="flex items-center gap-3">
        <Skeleton className="h-12 w-12 rounded-full" />
        <Skeleton className="h-6 w-40" />
      </div>
      <div className="grid overflow-hidden rounded-lg border border-border/70 sm:grid-cols-3">
        <Skeleton className="h-20 rounded-none" />
        <Skeleton className="h-20 rounded-none" />
        <Skeleton className="h-20 rounded-none" />
      </div>
      <Skeleton className="h-48 rounded-lg" />
    </div>
  );
}
