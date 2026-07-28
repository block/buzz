import {
  ChevronDown,
  ChevronUp,
  Maximize2,
  Minimize2,
  Network,
  Pencil,
  Plus,
  X,
} from "lucide-react";
import * as React from "react";
import { WorldViewTileSurface } from "@shivai.space/world-view-react";

import {
  useAuthorizeWorldViewMutation,
  useEffectiveWorldViewBindingsQuery,
  useLiveWorldViewBindingUpdates,
  useResolvedWorldViewQuery,
  useRevokeWorldViewMutation,
  useSetWorldViewBindingsMutation,
  useWorldViewBindingsQuery,
  useWorldAuthoritiesQuery,
  useWorldViewMutationDelegationsQuery,
} from "@/features/channels/hooks";
import { ChannelWorldViewBindingEditor } from "@/features/channels/ui/ChannelWorldViewBindingEditor";
import type {
  EffectiveWorldViewBinding,
  ResolvedWorldView,
  WorldViewBinding,
  WorldViewBindingScope,
  WorldMutationAuthority,
  WorldViewBindingsDocument,
} from "@/shared/api/worldViewTypes";
import { cn } from "@/shared/lib/cn";
import { useTheme } from "@/shared/theme/ThemeProvider";
import { Button } from "@/shared/ui/button";
import { Switch } from "@/shared/ui/switch";
import {
  AlertDialog,
  AlertDialogCancel,
  AlertDialogContent,
  AlertDialogDescription,
  AlertDialogFooter,
  AlertDialogHeader,
  AlertDialogTitle,
} from "@/shared/ui/alert-dialog";

export type ChannelWorldViewsProps = {
  channelId: string;
  canEdit: boolean;
  scope?: WorldViewBindingScope;
};

export function ChannelWorldViews({
  channelId,
  canEdit,
  scope = { kind: "channel" },
}: ChannelWorldViewsProps) {
  const threadRootEventId =
    scope.kind === "thread" ? scope.threadRootEventId : null;
  useLiveWorldViewBindingUpdates(channelId);
  const bindingsQuery = useWorldViewBindingsQuery(channelId, scope);
  const effectiveQuery = useEffectiveWorldViewBindingsQuery(
    channelId,
    threadRootEventId,
  );
  const mutation = useSetWorldViewBindingsMutation(channelId);
  const [expanded, setExpanded] = React.useState(true);
  const [isMaximized, setIsMaximized] = React.useState(false);
  const [editorTarget, setEditorTarget] = React.useState<{
    binding: WorldViewBinding | null;
  } | null>(null);
  const [removalTarget, setRemovalTarget] =
    React.useState<WorldViewBinding | null>(null);
  const document = bindingsQuery.data?.document;
  const bindings = document?.bindings ?? [];
  const bindingScope = document?.scope ?? scope;
  const effectiveEntries = effectiveQuery.data?.bindings ?? [];
  const editing = editorTarget !== null;
  function resetEditor(): void {
    setEditorTarget(null);
    setIsMaximized(false);
  }

  function beginEditing(binding: WorldViewBinding | null = null): void {
    setEditorTarget({ binding });
    setIsMaximized(true);
    setExpanded(true);
  }

  async function publishDocument(nextDocument: WorldViewBindingsDocument) {
    await mutation.mutateAsync({
      document: nextDocument,
      expectedRevisionEventId: bindingsQuery.data?.revisionEventId ?? null,
    });
  }
  async function removeBinding(binding: WorldViewBinding): Promise<void> {
    try {
      await publishDocument({
        version: 4,
        scope: bindingScope,
        bindings: bindings.filter((candidate) => candidate.id !== binding.id),
      });
      setRemovalTarget(null);
    } catch {
      // The mutation exposes its error in the confirmation dialog.
    }
  }

  if (bindingsQuery.isLoading || effectiveQuery.isLoading) {
    return (
      <div className="border-b border-border/70 px-5 py-2 text-xs text-muted-foreground">
        Loading Shivai views...
      </div>
    );
  }

  const queryError =
    bindingsQuery.error instanceof Error
      ? bindingsQuery.error
      : effectiveQuery.error instanceof Error
        ? effectiveQuery.error
        : null;
  if (queryError) {
    return (
      <div className="border-b border-destructive/30 bg-destructive/5 px-5 py-2 text-xs text-destructive">
        {queryError.message}
      </div>
    );
  }

  if (effectiveEntries.length === 0 && !editing) {
    if (!canEdit) return null;
    return (
      <div className="border-b border-border/70 bg-card/30 px-5 py-2">
        <Button
          className="h-7 gap-1.5 text-xs"
          onClick={() => beginEditing()}
          size="sm"
          variant="ghost"
        >
          <Network className="h-3.5 w-3.5" />
          Bind a Shivai world view
        </Button>
      </div>
    );
  }

  return (
    <section
      aria-label="Shivai world views"
      className={cn(
        "flex min-h-0 flex-col overflow-hidden border-b border-border/70 bg-card/25",
        !isMaximized &&
          "max-h-[max(12rem,calc(100dvh-var(--buzz-top-chrome-height,40px)-var(--composer-overlay-height,0px)-5rem))]",
        isMaximized &&
          "fixed inset-x-4 bottom-4 top-[calc(var(--buzz-top-chrome-height,40px)+1rem)] z-50 rounded-xl border bg-card shadow-2xl",
      )}
      data-maximized={isMaximized}
      data-testid="channel-world-views"
    >
      <div className="flex min-h-10 shrink-0 items-center justify-between gap-3 px-5 py-1.5">
        <button
          className="flex min-w-0 items-center gap-2 text-left text-xs font-semibold text-foreground"
          onClick={() => {
            if (expanded) {
              setIsMaximized(false);
            }
            setExpanded((current) => !current);
          }}
          type="button"
        >
          <Network className="h-3.5 w-3.5 shrink-0 text-primary" />
          <span className="truncate">Shivai world views</span>
          <span className="rounded-full bg-muted px-1.5 py-0.5 text-3xs font-medium text-muted-foreground">
            {effectiveEntries.length}
          </span>
          {expanded ? (
            <ChevronUp className="h-3.5 w-3.5 text-muted-foreground" />
          ) : (
            <ChevronDown className="h-3.5 w-3.5 text-muted-foreground" />
          )}
        </button>
        <div className="flex items-center gap-1">
          <Button
            aria-label={
              isMaximized
                ? "Restore Shivai world views"
                : "Expand Shivai world views"
            }
            className="h-7 gap-1 text-xs"
            onClick={() => {
              setExpanded(true);
              setIsMaximized((current) => !current);
            }}
            size="sm"
            variant="ghost"
          >
            {isMaximized ? (
              <Minimize2 className="h-3.5 w-3.5" />
            ) : (
              <Maximize2 className="h-3.5 w-3.5" />
            )}
            {isMaximized ? "Restore" : "Expand"}
          </Button>
          {canEdit ? (
            <Button
              className="h-7 gap-1 text-xs"
              onClick={() => {
                if (editing) {
                  resetEditor();
                } else {
                  beginEditing();
                }
              }}
              size="sm"
              variant="ghost"
            >
              {editing ? (
                <X className="h-3.5 w-3.5" />
              ) : (
                <Plus className="h-3.5 w-3.5" />
              )}
              {editing ? "Cancel" : "Add view"}
            </Button>
          ) : null}
        </div>
      </div>

      {expanded ? (
        <div className="flex min-h-0 flex-1 flex-col space-y-3 overflow-y-auto px-4 pb-4">
          {editorTarget ? (
            <ChannelWorldViewBindingEditor
              binding={editorTarget.binding}
              bindingScope={bindingScope}
              bindings={bindings}
              isPublishing={mutation.isPending}
              key={editorTarget.binding?.id ?? "new"}
              onComplete={resetEditor}
              onPublish={publishDocument}
              publishError={
                mutation.error instanceof Error ? mutation.error : null
              }
            />
          ) : null}

          {effectiveEntries.length > 0 ? (
            <div
              className={cn(
                "flex snap-x gap-3 overflow-x-auto pb-1",
                isMaximized && "min-h-0 flex-1",
              )}
            >
              {effectiveEntries.map((entry) => {
                const declaredHere =
                  bindingScope.kind === "channel"
                    ? entry.declaredScope.kind === "channel"
                    : entry.declaredScope.kind === "thread" &&
                      entry.declaredScope.threadRootEventId ===
                        bindingScope.threadRootEventId;
                return (
                  <div
                    className={cn(
                      "min-w-full flex-1 snap-start",
                      isMaximized && "flex min-h-0 flex-col",
                    )}
                    key={entry.binding.id}
                  >
                    <div className="flex min-h-7 items-center justify-between gap-2 px-1 pb-1 text-3xs text-muted-foreground">
                      <span>
                        {entry.declaredScope.kind === "channel"
                          ? bindingScope.kind === "thread"
                            ? "Inherited from channel"
                            : "Channel binding"
                          : "Thread override"}
                      </span>
                      {canEdit ? (
                        <div className="flex items-center gap-1">
                          <Button
                            aria-label={`${
                              declaredHere ? "Edit" : "Override"
                            } ${entry.binding.label ?? "world view"}`}
                            className="h-7 gap-1 px-2 text-3xs"
                            disabled={mutation.isPending}
                            onClick={() => beginEditing(entry.binding)}
                            size="sm"
                            variant="ghost"
                          >
                            <Pencil className="h-3 w-3" />
                            {declaredHere ? "Edit" : "Override"}
                          </Button>
                          {declaredHere ? (
                            <Button
                              aria-label={`Remove ${
                                entry.binding.label ?? "world view"
                              }`}
                              className="h-7 w-7 p-0"
                              disabled={mutation.isPending}
                              onClick={() => setRemovalTarget(entry.binding)}
                              size="icon"
                              variant="ghost"
                            >
                              <X className="h-3.5 w-3.5" />
                            </Button>
                          ) : null}
                        </div>
                      ) : null}
                    </div>
                    <div
                      className={cn(
                        "min-h-[28rem]",
                        isMaximized && "min-h-0 flex-1",
                      )}
                    >
                      <ChannelWorldViewTile
                        channelId={channelId}
                        effectiveScope={
                          effectiveQuery.data?.effectiveScope ?? bindingScope
                        }
                        entry={entry}
                      />
                    </div>
                  </div>
                );
              })}
            </div>
          ) : null}
        </div>
      ) : null}
      <WorldViewRemovalDialog
        binding={removalTarget}
        error={mutation.error instanceof Error ? mutation.error : null}
        isRemoving={mutation.isPending}
        onCancel={() => setRemovalTarget(null)}
        onRemove={(binding) => void removeBinding(binding)}
        scope={bindingScope}
      />
    </section>
  );
}
type WorldViewRemovalDialogProps = {
  binding: WorldViewBinding | null;
  error: Error | null;
  isRemoving: boolean;
  onCancel: () => void;
  onRemove: (binding: WorldViewBinding) => void;
  scope: WorldViewBindingScope;
};

function WorldViewRemovalDialog({
  binding,
  error,
  isRemoving,
  onCancel,
  onRemove,
  scope,
}: WorldViewRemovalDialogProps) {
  return (
    <AlertDialog
      onOpenChange={(open) => {
        if (!open && !isRemoving) {
          onCancel();
        }
      }}
      open={binding !== null}
    >
      <AlertDialogContent data-testid="world-view-remove-confirmation">
        <AlertDialogHeader>
          <AlertDialogTitle>Remove world-view binding?</AlertDialogTitle>
          <AlertDialogDescription>
            Remove <strong>{binding?.label ?? "this Shivai world view"}</strong>{" "}
            from this {scope.kind}. The Shivai world itself will remain
            unchanged.
          </AlertDialogDescription>
        </AlertDialogHeader>
        {error ? (
          <p className="text-sm text-destructive">{error.message}</p>
        ) : null}
        <AlertDialogFooter>
          <AlertDialogCancel asChild>
            <Button disabled={isRemoving} type="button" variant="outline">
              Keep binding
            </Button>
          </AlertDialogCancel>
          <Button
            data-testid="world-view-remove-confirm"
            disabled={isRemoving}
            onClick={() => {
              if (binding) {
                onRemove(binding);
              }
            }}
            type="button"
            variant="destructive"
          >
            {isRemoving ? "Removing..." : "Remove binding"}
          </Button>
        </AlertDialogFooter>
      </AlertDialogContent>
    </AlertDialog>
  );
}

type ChannelWorldViewTileProps = {
  channelId: string;
  effectiveScope: WorldViewBindingScope;
  entry: EffectiveWorldViewBinding;
};

function ChannelWorldViewTile({
  channelId,
  effectiveScope,
  entry,
}: ChannelWorldViewTileProps) {
  const { isDark } = useTheme();
  const resolutionRequest = {
    binding: entry.binding,
    bindingRevisionEventId: entry.bindingRevisionEventId,
    channelId,
    declaredScope: entry.declaredScope,
    effectiveScope,
  };
  const resolvedQuery = useResolvedWorldViewQuery(resolutionRequest);

  if (resolvedQuery.isLoading) {
    return (
      <div className="grid h-full place-items-center rounded-2xl border border-border/70 bg-background/70 text-xs text-muted-foreground">
        Resolving Shivai world view...
      </div>
    );
  }

  if (resolvedQuery.error instanceof Error) {
    return (
      <div className="grid h-full place-items-center rounded-2xl border border-destructive/30 bg-destructive/5 p-6 text-center">
        <div className="max-w-md space-y-3">
          <strong className="text-sm text-destructive">
            World view unavailable
          </strong>
          <p className="text-xs leading-relaxed text-muted-foreground">
            {resolvedQuery.error.message}
          </p>
          <Button
            onClick={() => void resolvedQuery.refetch()}
            size="sm"
            variant="outline"
          >
            Retry
          </Button>
        </div>
      </div>
    );
  }

  if (!resolvedQuery.data) return null;
  const freshnessLabel =
    resolvedQuery.data.freshness === "pinned"
      ? "Pinned export"
      : resolvedQuery.data.authority.kind === "hosted-world-latest"
        ? "Latest hosted world"
        : resolvedQuery.data.authority.kind === "hosted-world-live-view-share"
          ? "Latest shared view"
          : "Latest mirror";
  return (
    <div className="flex h-full min-h-0 flex-col gap-2">
      <ChannelWorldViewMutationControl
        authority={toWorldMutationAuthority(resolvedQuery.data.authority)}
        channelId={channelId}
        entry={entry}
      />
      <div className="min-h-0 flex-1">
        <WorldViewTileSurface
          appearance={isDark ? "dark" : "light"}
          defaultDisplayMode={entry.binding.displayMode}
          isRefreshing={resolvedQuery.isFetching}
          onRefresh={async () => {
            await resolvedQuery.refetch();
          }}
          presentation={resolvedQuery.data.presentation}
          subtitle={`${freshnessLabel} · ${resolvedQuery.data.realm.qualifiedName}`}
          title={entry.binding.label ?? resolvedQuery.data.view.name}
        />
      </div>
    </div>
  );
}

type ResolvedWorldViewAuthority = ResolvedWorldView["authority"];

function toWorldMutationAuthority(
  authority: ResolvedWorldViewAuthority,
): WorldMutationAuthority | null {
  switch (authority.kind) {
    case "local-world-mirror-latest":
    case "hosted-world-latest":
      return authority;
    case "hosted-world-live-view-share":
      return {
        kind: "hosted-world-latest",
        origin: authority.origin,
        hostedWorldId: authority.hostedWorldId,
      };
    case "hosted-world-view-export":
      return null;
  }
}

type ChannelWorldViewMutationControlProps = {
  authority: WorldMutationAuthority | null;
  channelId: string;
  entry: EffectiveWorldViewBinding;
};

function ChannelWorldViewMutationControl({
  authority,
  channelId,
  entry,
}: ChannelWorldViewMutationControlProps) {
  const authoritiesQuery = useWorldAuthoritiesQuery();
  const delegationsQuery = useWorldViewMutationDelegationsQuery();
  const authorizeMutation = useAuthorizeWorldViewMutation();
  const revokeMutation = useRevokeWorldViewMutation();
  const connected =
    authority !== null &&
    (authoritiesQuery.data?.authorities ?? []).some((candidate) =>
      worldMutationAuthoritiesEqual(candidate, authority),
    );
  const enabled =
    authority !== null &&
    (delegationsQuery.data?.delegations ?? []).some(
      (delegation) =>
        delegation.channelId === channelId &&
        delegation.bindingId === entry.binding.id &&
        worldViewBindingScopesEqual(
          delegation.declaredScope,
          entry.declaredScope,
        ) &&
        delegation.bindingRevisionEventId === entry.bindingRevisionEventId &&
        worldMutationAuthoritiesEqual(delegation.authority, authority),
    );
  const isPending =
    authoritiesQuery.isLoading ||
    delegationsQuery.isLoading ||
    authorizeMutation.isPending ||
    revokeMutation.isPending;
  const mutationError =
    authorizeMutation.error instanceof Error
      ? authorizeMutation.error
      : revokeMutation.error instanceof Error
        ? revokeMutation.error
        : authoritiesQuery.error instanceof Error
          ? authoritiesQuery.error
          : delegationsQuery.error instanceof Error
            ? delegationsQuery.error
            : null;
  const availabilityLabel = authority
    ? enabled
      ? connected
        ? "Agents can use a scoped command; paths and credentials stay private."
        : "Agent edit consent is on, but this source is no longer connected."
      : connected
        ? "Read-only for agents until you enable this binding."
        : "Connect this world source on this device to enable agent edits."
    : "Published view exports are read-only.";

  return (
    <div className="flex min-h-9 items-center justify-between gap-3 rounded-lg border border-border/70 bg-background/70 px-3 py-1.5">
      <div className="min-w-0">
        <div className="text-xs font-medium text-foreground">
          Allow agents to edit
        </div>
        <div className="truncate text-3xs text-muted-foreground">
          {availabilityLabel}
        </div>
        {mutationError ? (
          <div className="text-3xs text-destructive">
            {mutationError.message}
          </div>
        ) : null}
      </div>
      <Switch
        aria-label={`Allow agents to edit ${
          entry.binding.label ?? "this world view"
        }`}
        checked={enabled}
        disabled={isPending || authority === null || (!connected && !enabled)}
        onCheckedChange={(checked) => {
          if (!authority) return;
          authorizeMutation.reset();
          revokeMutation.reset();
          if (checked) {
            authorizeMutation.mutate({
              authority,
              bindingId: entry.binding.id,
              bindingRevisionEventId: entry.bindingRevisionEventId,
              channelId,
              declaredScope: entry.declaredScope,
            });
          } else {
            revokeMutation.mutate({
              bindingId: entry.binding.id,
              channelId,
              declaredScope: entry.declaredScope,
            });
          }
        }}
      />
    </div>
  );
}

function worldViewBindingScopesEqual(
  left: WorldViewBindingScope,
  right: WorldViewBindingScope,
): boolean {
  if (left.kind === "channel") return right.kind === "channel";
  return (
    right.kind === "thread" &&
    left.threadRootEventId === right.threadRootEventId
  );
}

function worldMutationAuthoritiesEqual(
  left: WorldMutationAuthority,
  right: WorldMutationAuthority,
): boolean {
  if (left.origin !== right.origin) return false;
  if (left.kind === "local-world-mirror-latest") {
    return (
      right.kind === "local-world-mirror-latest" &&
      left.mirrorId === right.mirrorId
    );
  }
  return (
    right.kind === "hosted-world-latest" &&
    left.hostedWorldId === right.hostedWorldId
  );
}
