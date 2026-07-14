import { isCatalogPersonaSelected } from "@/features/agents/lib/catalog";
import { ProfileAvatar } from "@/features/profile/ui/ProfileAvatar";
import type { AgentPersona } from "@/shared/api/types";
import { cn } from "@/shared/lib/cn";
import { promptPreview } from "@/shared/lib/promptPreview";
import {
  Sheet,
  SheetContent,
  SheetDescription,
  SheetHeader,
  SheetTitle,
} from "@/shared/ui/sheet";

import { PersonaCatalogSelectionBadge } from "./PersonaCatalogSelectionBadge";
import {
  getPersonaCatalogDetailSelectionCopy,
  getPersonaCatalogSelectionAriaLabel,
} from "./personaLibraryCopy";

type PersonaCatalogDetailsSheetProps = {
  feedbackErrorMessage: string | null;
  feedbackNoticeMessage: string | null;
  isPending: boolean;
  onOpenChange: (open: boolean) => void;
  onTogglePersona: (persona: AgentPersona) => void;
  open: boolean;
  persona: AgentPersona | null;
};

export function PersonaCatalogDetailsSheet({
  feedbackErrorMessage,
  feedbackNoticeMessage,
  isPending,
  onOpenChange,
  onTogglePersona,
  open,
  persona,
}: PersonaCatalogDetailsSheetProps) {
  const preview = persona ? promptPreview(persona.systemPrompt) : "";
  const isSelected = persona ? isCatalogPersonaSelected(persona) : false;
  const selectionCopy = getPersonaCatalogDetailSelectionCopy(isSelected);

  return (
    <Sheet onOpenChange={onOpenChange} open={open}>
      <SheetContent
        className="flex w-full flex-col gap-0 overflow-hidden bg-background p-0 sm:max-w-xl"
        data-testid="persona-catalog-details-sheet"
      >
        {persona ? (
          <>
            <SheetHeader className="relative z-10 space-y-4 bg-background/25 px-6 py-6 pr-16 text-left shadow-[0_4px_24px_rgba(0,0,0,0.06)] backdrop-blur-xl supports-[backdrop-filter]:bg-background/20 dark:shadow-[0_4px_24px_rgba(0,0,0,0.25)]">
              <div className="flex items-start gap-3">
                <ProfileAvatar
                  avatarUrl={persona.avatarUrl}
                  className="h-12 w-12 text-sm"
                  label={persona.displayName}
                />
                <div className="min-w-0 flex-1">
                  <div className="flex flex-wrap items-center gap-2">
                    <SheetTitle className="truncate text-xl">
                      {persona.displayName}
                    </SheetTitle>
                    <PersonaCatalogSelectionBadge isActive={isSelected} />
                  </div>
                  <SheetDescription className="mt-2">
                    {preview || "No summary available."}
                  </SheetDescription>
                </div>
              </div>
            </SheetHeader>
            <div className="flex-1 space-y-6 overflow-y-auto px-6 py-6">
              <button
                aria-label={getPersonaCatalogSelectionAriaLabel(
                  persona.displayName,
                  isSelected,
                )}
                aria-pressed={isSelected}
                className={cn(
                  "w-full rounded-xl border p-4 text-left transition-[background-color,border-color,box-shadow] focus:outline-hidden focus-visible:ring-2 focus-visible:ring-primary/40 focus-visible:ring-offset-2",
                  isSelected
                    ? "border-primary bg-primary/10 text-foreground"
                    : "border-border/80 bg-background/60 text-muted-foreground hover:bg-accent hover:text-accent-foreground",
                  isPending && "cursor-not-allowed opacity-70",
                )}
                data-state={isSelected ? "selected" : "available"}
                data-testid={`persona-catalog-detail-selection-target-${persona.id}`}
                disabled={isPending}
                onClick={() => {
                  onTogglePersona(persona);
                }}
                type="button"
              >
                <div className="flex items-start justify-between gap-4">
                  <div>
                    <p
                      className="text-sm font-semibold tracking-tight"
                      data-testid="persona-catalog-detail-selection-title"
                    >
                      {selectionCopy.title}
                    </p>
                    <p
                      className="mt-1 text-sm text-muted-foreground"
                      data-testid="persona-catalog-detail-selection-description"
                    >
                      {selectionCopy.description}
                    </p>
                  </div>
                  <PersonaCatalogSelectionBadge isActive={isSelected} />
                </div>
              </button>

              {feedbackNoticeMessage ? (
                <p className="rounded-2xl border border-primary/20 bg-primary/10 px-4 py-3 text-sm text-primary">
                  {feedbackNoticeMessage}
                </p>
              ) : null}

              {feedbackErrorMessage ? (
                <p className="rounded-2xl border border-destructive/30 bg-destructive/10 px-4 py-3 text-sm text-destructive">
                  {feedbackErrorMessage}
                </p>
              ) : null}

              <div className="grid gap-3 sm:grid-cols-2">
                <div className="rounded-xl border border-border/70 bg-card/70 p-4">
                  <p className="text-xs font-semibold uppercase tracking-[0.14em] text-muted-foreground">
                    Type
                  </p>
                  <p className="mt-2 text-sm font-medium">Built-in agent</p>
                </div>
                <div className="rounded-xl border border-border/70 bg-card/70 p-4">
                  <p className="text-xs font-semibold uppercase tracking-[0.14em] text-muted-foreground">
                    Preferred model
                  </p>
                  <p className="mt-2 text-sm font-medium">
                    {persona.model ?? "Use app default"}
                  </p>
                </div>
                <div className="rounded-xl border border-border/70 bg-card/70 p-4 sm:col-span-2">
                  <p className="text-xs font-semibold uppercase tracking-[0.14em] text-muted-foreground">
                    Preferred runtime
                  </p>
                  <p className="mt-2 text-sm font-medium">
                    {persona.runtime ?? "Use app default"}
                  </p>
                </div>
              </div>

              <div className="rounded-xl border border-border/70 bg-card/70 p-4">
                <p className="text-xs font-semibold uppercase tracking-[0.14em] text-muted-foreground">
                  System prompt
                </p>
                <pre className="mt-3 whitespace-pre-wrap break-words font-sans text-sm leading-6 text-foreground">
                  {persona.systemPrompt}
                </pre>
              </div>
            </div>
          </>
        ) : null}
      </SheetContent>
    </Sheet>
  );
}
