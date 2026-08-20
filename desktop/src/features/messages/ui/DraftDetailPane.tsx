import {
  AlertTriangle,
  ArrowLeft,
  FileText,
  Lock,
  Pencil,
  Send,
  Trash2,
} from "lucide-react";
import * as React from "react";

import { useAppNavigation } from "@/app/navigation/useAppNavigation";
import {
  canOpenDraft,
  canSendDraft,
  formatDraftCreatedAt,
  getDraftPreview,
  openDraftEntry,
  SendConfirmDialog,
  sendDraftEntry,
  type DraftViewItem,
} from "@/features/messages/ui/DraftsPanel";
import { TopChromeInsetHeader } from "@/shared/layout/TopChromeInsetHeader";
import { Button } from "@/shared/ui/button";
import { Markdown } from "@/shared/ui/markdown";
import { Tooltip, TooltipContent, TooltipTrigger } from "@/shared/ui/tooltip";
import { UserAvatar } from "@/shared/ui/UserAvatar";

type DraftDetailPaneProps = {
  item: DraftViewItem | null;
  onBack?: () => void;
  onDelete: (draftKey: string) => void;
};

export function DraftDetailPane({
  item,
  onBack,
  onDelete,
}: DraftDetailPaneProps) {
  const { goChannel } = useAppNavigation();
  const [sendDialogOpen, setSendDialogOpen] = React.useState(false);

  if (!item) {
    return (
      <section
        className="flex min-h-0 min-w-0 items-center justify-center bg-background/60 px-6 py-10 pt-20 text-center"
        data-testid="home-inbox-draft-detail-empty"
      >
        <div className="max-w-sm">
          <div className="mx-auto flex h-14 w-14 items-center justify-center rounded-full bg-muted text-muted-foreground">
            <FileText className="h-6 w-6" />
          </div>
          <p className="mt-4 text-base font-semibold">Select a draft</p>
          <p className="mt-1 text-sm text-muted-foreground">
            Pick a draft to preview it and choose what to do next.
          </p>
        </div>
      </section>
    );
  }

  const { entry, rootStatus, source } = item;
  const isOrphaned = rootStatus === "deleted";
  const isPrivate = source.channel?.visibility === "private";
  const isDm = source.channel?.channelType === "dm";
  const channelLabel = source.channel
    ? isDm
      ? source.label
      : `#${source.label}`
    : "Unknown channel";
  const openEnabled = canOpenDraft(entry.draft, source) && !isOrphaned;
  const sendEnabled = canSendDraft(entry.draft, source, rootStatus);
  const content = entry.draft.content.trim();
  const attachmentCount = entry.draft.pendingImeta.length;

  return (
    <section
      className="flex min-h-0 min-w-0 flex-col overflow-hidden bg-background/60"
      data-testid="home-inbox-draft-detail"
    >
      <TopChromeInsetHeader flush transparent>
        <div className="px-5 py-2">
          <div className="flex min-h-9 min-w-0 items-center justify-between gap-3">
            <div className="flex min-w-0 items-center gap-1">
              {onBack ? (
                <Button
                  aria-label="Back to drafts list"
                  className="rounded-full text-muted-foreground hover:bg-muted/60 hover:text-foreground"
                  onClick={onBack}
                  size="icon"
                  type="button"
                  variant="ghost"
                >
                  <ArrowLeft />
                </Button>
              ) : null}
              <div className="flex min-w-0 items-center gap-1.5">
                {isPrivate ? (
                  <Lock className="h-3.5 w-3.5 shrink-0 text-muted-foreground" />
                ) : null}
                <h2 className="truncate text-sm font-semibold leading-5 tracking-tight text-foreground">
                  {channelLabel}
                </h2>
              </div>
            </div>
          </div>
        </div>
      </TopChromeInsetHeader>

      <div className="-mt-13 min-h-0 flex-1 overflow-y-auto pb-8 pt-15">
        {isOrphaned ? (
          <div
            className="mx-5 mb-3 flex items-start gap-2 rounded-lg border border-destructive/25 bg-destructive/5 p-3 text-sm text-destructive"
            data-testid="home-inbox-draft-orphaned-notice"
          >
            <AlertTriangle className="mt-0.5 h-4 w-4 shrink-0" />
            <span>
              The original thread was deleted. This draft can no longer be
              opened or sent.
            </span>
          </div>
        ) : null}

        <div className="relative px-2">
          <article className="group/message relative z-10 mx-1 flex items-start gap-2.5 rounded-2xl px-2 py-1 transition-colors hover:bg-muted/50 focus-within:bg-muted/50">
            <DraftActionBar
              canOpen={openEnabled}
              canSend={sendEnabled}
              onDelete={() => onDelete(entry.key)}
              onOpen={() => void openDraftEntry(entry, goChannel)}
              onSend={() => setSendDialogOpen(true)}
            />

            <UserAvatar
              avatarUrl={null}
              className="h-9 w-9 shrink-0"
              displayName="You"
              size="md"
            />

            <div className="min-w-0 flex-1">
              <div className="flex min-w-0 flex-wrap items-center gap-x-2 gap-y-0">
                <span className="text-sm font-semibold text-foreground">
                  You
                </span>
                <span className="text-xs font-medium text-muted-foreground">
                  Draft
                </span>
                <span className="shrink-0 text-xs font-normal tabular-nums text-muted-foreground/55">
                  {formatDraftCreatedAt(entry.draft)}
                </span>
              </div>

              <div className="mt-0.5 text-base leading-6 text-foreground">
                <Markdown
                  className="inbox-preview-markdown text-inherit leading-6"
                  content={content || getDraftPreview(entry.draft)}
                  interactive={false}
                />
                {attachmentCount > 0 && content ? (
                  <p className="mt-2 text-sm text-muted-foreground">
                    {attachmentCount} attachment
                    {attachmentCount === 1 ? "" : "s"}
                  </p>
                ) : null}
              </div>
            </div>
          </article>
        </div>
      </div>

      {sendDialogOpen ? (
        <SendConfirmDialog
          channelLabel={source.label}
          isDm={isDm}
          onCancel={() => setSendDialogOpen(false)}
          onConfirm={() => {
            setSendDialogOpen(false);
            void sendDraftEntry(entry, goChannel);
          }}
          open={true}
        />
      ) : null}
    </section>
  );
}

function DraftActionBar({
  canOpen,
  canSend,
  onDelete,
  onOpen,
  onSend,
}: {
  canOpen: boolean;
  canSend: boolean;
  onDelete: () => void;
  onOpen: () => void;
  onSend: () => void;
}) {
  return (
    <div className="absolute right-2 top-1 z-10">
      <div
        className="-m-1 p-1 opacity-100 transition-opacity duration-150 ease-out sm:pointer-events-none sm:opacity-0 sm:group-hover/message:pointer-events-auto sm:group-hover/message:opacity-100 sm:group-focus-within/message:pointer-events-auto sm:group-focus-within/message:opacity-100"
        data-testid="home-inbox-draft-action-bar"
      >
        <div className="overflow-hidden rounded-full border border-border/70 bg-background/95 shadow-xs backdrop-blur-sm supports-[backdrop-filter]:bg-background/85">
          <div className="flex items-center gap-0.5 p-1">
            <DraftActionButton
              disabled={!canOpen}
              label="Open draft"
              onClick={onOpen}
            >
              <Pencil className="h-4 w-4" />
            </DraftActionButton>
            <DraftActionButton
              disabled={!canSend}
              label="Send"
              onClick={onSend}
            >
              <Send className="h-4 w-4" />
            </DraftActionButton>
            <DraftActionButton destructive label="Delete" onClick={onDelete}>
              <Trash2 className="h-4 w-4" />
            </DraftActionButton>
          </div>
        </div>
      </div>
    </div>
  );
}

function DraftActionButton({
  children,
  destructive = false,
  disabled = false,
  label,
  onClick,
}: {
  children: React.ReactNode;
  destructive?: boolean;
  disabled?: boolean;
  label: string;
  onClick: () => void;
}) {
  return (
    <Tooltip>
      <TooltipTrigger asChild>
        <Button
          aria-label={label}
          className={
            destructive
              ? "h-8 w-8 rounded-full p-0 text-destructive hover:text-destructive"
              : "h-8 w-8 rounded-full p-0"
          }
          disabled={disabled}
          onClick={onClick}
          size="sm"
          type="button"
          variant="ghost"
        >
          {children}
        </Button>
      </TooltipTrigger>
      <TooltipContent>{label}</TooltipContent>
    </Tooltip>
  );
}
