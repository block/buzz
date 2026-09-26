import { AppWindow, ExternalLink, Globe, Plus, Trash2 } from "lucide-react";
import * as React from "react";
import { useSyncExternalStore } from "react";
import { toast } from "sonner";

import { useAppNavigation } from "@/app/navigation/useAppNavigation";
import { BrowserAgentChrome } from "@/features/browser-agent/ui/BrowserAgentChrome";
import {
  listBrowserAgentGrants,
  subscribeBrowserAgentGrant,
} from "@/features/browser-agent/lib/api";
import type { BrowserAgentGrant } from "@/features/browser-agent/lib/types";
import { useChannelReferences } from "@/features/channels/openChannelDirectory";
import { usePlaygroundSessions } from "@/features/playground/hooks";
import {
  getPlaygroundViewport,
  getPlaygroundViewportRevision,
  playgroundViewportCaption,
  subscribePlaygroundViewport,
} from "@/features/playground/lib/playgroundViewport";
import {
  PLAYGROUND_HULA,
  PLAYGROUND_VERSION,
  type PlaygroundCard,
} from "@/features/playground/lib/types";
import {
  getConversationPlaygroundPinsRevision,
  listConversationPinBindingsForSid,
  subscribeConversationPlaygroundPins,
} from "@/features/playground/lib/conversationPins";
import {
  addPlaygroundSession,
  showPlaygroundSession,
  switchPlaygroundTab,
} from "@/features/playground/lib/sessions";
import { isPlaygroundWebviewOpen } from "@/features/playground/lib/webview";
import {
  getEmbeddedWindowsStore,
  subscribeEmbeddedWindows,
} from "@/features/popout/lib/embeddedWindows";
import {
  openPopoutWindow,
  popoutErrorMessage,
  readPopoutPayload,
} from "@/features/popout/lib/popoutWindow";
import {
  getPopoutWindows,
  popoutKindFromLabel,
  subscribePopoutWindows,
} from "@/features/popout/lib/popoutWindows";
import { Badge } from "@/shared/ui/badge";
import { Button } from "@/shared/ui/button";
import { PageHeader } from "@/shared/ui/PageHeader";
import { cn } from "@/shared/lib/cn";

import {
  type BrowserConversationBinding,
  browserBindingChipLabel,
  browserBindingChipTooltip,
  buildBrowserConversationBindings,
} from "../lib/browserBindings";
import { disposeBrowserSession } from "../lib/disposeBrowserSession";
import { sidRunbookRef } from "@/features/site-runbook/lib/keys";
import { SiteRunbookOpenButton } from "@/features/site-runbook/ui/SiteRunbookDialog";
import {
  type BrowserListRow,
  type DetachedBrowserHost,
  browserRowLivenessChip,
  browserRowShowsOpenButton,
  browserRowUrlLabel,
  buildBrowserListRows,
  findGrantForRow,
} from "../lib/browserRows";
import { BROWSER_PREVIEW_REFRESH_MS } from "../lib/browserPreview";
import { AddBrowserDialog } from "./AddBrowserDialog";
import { BrowserRowPreview } from "./BrowserRowPreview";

function useDetachedBrowserHosts(): DetachedBrowserHost[] {
  const osRows = useSyncExternalStore(
    subscribePopoutWindows,
    getPopoutWindows,
    getPopoutWindows,
  );
  const embedded = useSyncExternalStore(
    subscribeEmbeddedWindows,
    getEmbeddedWindowsStore,
    getEmbeddedWindowsStore,
  );

  return React.useMemo(() => {
    const hosts: DetachedBrowserHost[] = [];

    for (const row of osRows) {
      const kind = popoutKindFromLabel(row.label);
      if (kind !== "playground" && kind !== "split") continue;
      const payload = readPopoutPayload(row.label);
      const playgroundSid = payload?.playground?.sid;
      if (!playgroundSid) continue;
      hosts.push({
        label: row.label,
        title: row.title,
        playgroundSid,
      });
    }

    for (const row of embedded.windows) {
      const kind = row.payload.kind;
      if (kind !== "playground" && kind !== "split") continue;
      const playgroundSid = row.payload.playground?.sid;
      if (!playgroundSid) continue;
      hosts.push({
        label: row.label,
        title: row.title,
        playgroundSid,
      });
    }

    return hosts;
  }, [embedded.windows, osRows]);
}

function useBrowserAgentGrants(): BrowserAgentGrant[] {
  const [grants, setGrants] = React.useState<BrowserAgentGrant[]>([]);

  const refresh = React.useCallback(() => {
    void listBrowserAgentGrants()
      .then(setGrants)
      .catch(() => setGrants([]));
  }, []);

  React.useEffect(() => {
    refresh();
    const unlisten = subscribeBrowserAgentGrant(() => {
      refresh();
    });
    return () => {
      void unlisten.then((stop) => stop());
    };
  }, [refresh]);

  return grants;
}

function useConversationPinsEpoch(): number {
  return useSyncExternalStore(
    subscribeConversationPlaygroundPins,
    getConversationPlaygroundPinsRevision,
    getConversationPlaygroundPinsRevision,
  );
}


/** Poll native WKWebView existence; null until first check (avoids Cold flash on live). */
function useBrowserRowWebviewOpen(
  sid: string,
  windowLabel: string,
): boolean | null {
  const [open, setOpen] = React.useState<boolean | null>(null);
  React.useEffect(() => {
    let cancelled = false;
    setOpen(null);
    async function refresh() {
      const next = await isPlaygroundWebviewOpen(sid, windowLabel);
      if (!cancelled) setOpen(next);
    }
    void refresh();
    const timer = window.setInterval(() => {
      void refresh();
    }, BROWSER_PREVIEW_REFRESH_MS);
    return () => {
      cancelled = true;
      window.clearInterval(timer);
    };
  }, [sid, windowLabel]);
  return open;
}

function BrowserRowColdChip({
  sid,
  windowLabel,
}: {
  sid: string;
  windowLabel: string;
}) {
  const open = useBrowserRowWebviewOpen(sid, windowLabel);
  // Only show after a confirmed miss — unknown/live omit the chip.
  const label = open === false ? browserRowLivenessChip(false) : null;
  if (!label) return null;
  return (
    <Badge
      className="normal-case tracking-normal"
      data-testid={`browser-row-cold-${sid}`}
      title="Webview not started — Open to launch"
      variant="secondary"
    >
      {label}
    </Badge>
  );
}

function BindingChips({
  bindings,
  channelNames,
  onOpenChannel,
  onOpenThread,
}: {
  bindings: BrowserConversationBinding[];
  channelNames: ReadonlyMap<string, string>;
  onOpenChannel: (channelId: string) => void;
  onOpenThread: (channelId: string, threadRoot: string) => void;
}) {
  if (bindings.length === 0) return null;
  return (
    <div
      className="mt-1.5 flex min-w-0 flex-wrap items-center gap-1"
      data-testid="browser-row-bindings"
    >
      {bindings.map((binding) => {
        const name = channelNames.get(binding.channelId);
        const label = browserBindingChipLabel(
          binding.channelId,
          name,
          binding.threadRoot,
        );
        const tooltip = browserBindingChipTooltip(
          binding.channelId,
          name,
          binding.threadRoot,
        );
        return (
          <button
            className={cn(
              "inline-flex max-w-[14rem] items-center truncate rounded-full border border-border/70",
              "bg-muted/50 px-2 py-0.5 text-2xs font-medium text-foreground/90",
              "hover:bg-muted hover:text-foreground",
            )}
            data-testid={`browser-row-binding-${binding.key}`}
            key={binding.key}
            onClick={(event) => {
              event.preventDefault();
              event.stopPropagation();
              if (binding.threadRoot) {
                onOpenThread(binding.channelId, binding.threadRoot);
              } else {
                onOpenChannel(binding.channelId);
              }
            }}
            title={tooltip}
            type="button"
          >
            {label}
          </button>
        );
      })}
    </div>
  );
}

export function BrowsersScreen() {
  const { goChannel } = useAppNavigation();
  const playground = usePlaygroundSessions();
  const sessions = React.useMemo(
    () => [...playground.sessions.values()],
    [playground.sessions],
  );
  const detached = useDetachedBrowserHosts();
  const grants = useBrowserAgentGrants();
  const pinsEpoch = useConversationPinsEpoch();
  const viewportEpoch = useSyncExternalStore(
    subscribePlaygroundViewport,
    getPlaygroundViewportRevision,
    getPlaygroundViewportRevision,
  );
  const embedded = useSyncExternalStore(
    subscribeEmbeddedWindows,
    getEmbeddedWindowsStore,
    getEmbeddedWindowsStore,
  );
  const [addOpen, setAddOpen] = React.useState(false);
  const [pendingRemoveKey, setPendingRemoveKey] = React.useState<string | null>(
    null,
  );

  const browsers = React.useMemo(
    () => [...playground.browsers.values()],
    [playground.browsers],
  );
  const rows = React.useMemo(
    () =>
      buildBrowserListRows({
        sessions,
        browsers,
        detached,
      }),
    [browsers, detached, sessions],
  );

  const rowBindings = React.useMemo(() => {
    void pinsEpoch;
    void viewportEpoch;
    return rows.map((row) => {
      const grant = findGrantForRow(grants, row);
      const pinBindingsRaw = listConversationPinBindingsForSid(row.surfaceId);
      const pinBindings = pinBindingsRaw.map((binding) => {
        if (binding.channelId) return binding;
        if (
          grant?.channelId &&
          binding.threadRoot &&
          grant.threadRoot === binding.threadRoot
        ) {
          return { ...binding, channelId: grant.channelId };
        }
        return binding;
      });
      return {
        row,
        grant,
        bindings: buildBrowserConversationBindings({
          grant,
          pinBindings,
        }),
      };
    });
  }, [grants, pinsEpoch, rows, viewportEpoch]);

  const channelIds = React.useMemo(
    () =>
      rowBindings.flatMap(({ bindings }) =>
        bindings.map((binding) => binding.channelId),
      ),
    [rowBindings],
  );
  const { channelsById } = useChannelReferences(channelIds);
  const channelNames = React.useMemo(() => {
    const names = new Map<string, string>();
    for (const [id, channel] of channelsById) {
      const name = channel.name?.trim();
      if (name) names.set(id, name);
    }
    return names;
  }, [channelsById]);

  function isEmbeddedLabel(label: string): boolean {
    return embedded.windows.some((row) => row.label === label);
  }

  function openAddedBrowser(card: PlaygroundCard) {
    addPlaygroundSession(card, { preferSidePanel: true });
    showPlaygroundSession(card.sid, { preferSidePanel: true });
  }

  /** Main-host Open — RHS slide-out. Detached rows have no Open path. */
  function openRow(row: BrowserListRow) {
    if (row.host !== "main") return;
    showPlaygroundSession(row.mainSurfaceId, { preferSidePanel: true });
  }

  /** Secondary subrow — switch tab only for the main-host browser. */
  function openSecondaryTab(row: BrowserListRow, surfaceId: string) {
    if (row.host !== "main") return;
    switchPlaygroundTab(surfaceId, { preferSidePanel: true });
  }

  async function removeRow(row: BrowserListRow) {
    try {
      await disposeBrowserSession({
        row,
        grants,
        isEmbeddedLabel,
      });
    } catch (error) {
      toast.error(
        error instanceof Error ? error.message : "Could not remove browser.",
      );
    }
  }

  async function detachRow(row: BrowserListRow) {
    if (row.host === "windowed") return;
    try {
      const session = playground.sessions.get(row.mainSurfaceId) ?? playground.sessions.get(row.surfaceId);
      if (!session) return;
      await openPopoutWindow({
        kind: "playground",
        title: session.name,
        seed: session.sid,
        playground: {
          hula: PLAYGROUND_HULA,
          v: PLAYGROUND_VERSION,
          name: session.name,
          url: session.url,
          sid: session.sid,
          ...(session.pin ? { pin: session.pin } : {}),
          ...(session.stack ? { stack: session.stack } : {}),
          ...(session.expires != null ? { expires: session.expires } : {}),
        },
      });
    } catch (error) {
      toast.error(popoutErrorMessage(error, "Could not detach browser."));
    }
  }

  return (
    <div className="flex-1 overflow-y-auto overflow-x-hidden overscroll-contain px-4 py-7 sm:px-6 sm:py-8">
      <div
        className="mx-auto w-full max-w-6xl space-y-8"
        data-testid="browsers-page-content"
      >
        <PageHeader
          action={
            <Button
              data-testid="browsers-add"
              onClick={() => setAddOpen(true)}
              size="sm"
              type="button"
            >
              <Plus className="mr-1 h-4 w-4" />
              Add
            </Button>
          }
          description="Playground and agent-driven browsers in this client, including detached windows."
          title="Browsers"
        />

        {rows.length === 0 ? (
          <div
            className="rounded-lg border border-dashed border-border/70 px-6 py-12 text-center"
            data-testid="browsers-empty"
          >
            <Globe className="mx-auto h-8 w-8 text-muted-foreground" />
            <p className="mt-3 text-sm font-medium">No browsers open yet</p>
            <p className="mt-1 text-sm text-muted-foreground">
              Use Add to open a site, or open a playground Site from a channel —
              then grant an agent Observe or Drive from the Bot toggle.
            </p>
            <Button
              className="mt-4"
              data-testid="browsers-empty-add"
              onClick={() => setAddOpen(true)}
              size="sm"
              type="button"
            >
              <Plus className="mr-1 h-4 w-4" />
              Add browser
            </Button>
          </div>
        ) : (
          <ul className="divide-y divide-border/60 rounded-lg border border-border/70">
            {rowBindings.map(({ row, bindings }) => (
              <li
                className="flex flex-col gap-2 px-4 py-3"
                data-testid={`browser-row-${row.key}`}
                key={row.key}
              >
                <BrowserAgentChrome
                  playgroundCard={{
                    hula: PLAYGROUND_HULA,
                    v: PLAYGROUND_VERSION,
                    name: row.title,
                    url: row.url,
                    sid: row.mainSurfaceId,
                  }}
                  showTakeRelease={false}
                  surface="playground"
                  surfaceId={row.surfaceId}
                  variant="toolbar"
                  windowLabel={row.windowLabel}
                >
                  {(agentToggle) => (
                    <div className="flex min-w-0 flex-col gap-2">
                    <div className="flex min-w-0 items-start gap-3">
                      <BrowserRowPreview
                        key={`${row.mainSurfaceId}:${row.url}`}
                        onOpen={() => void openRow(row)}
                        sid={row.mainSurfaceId}
                        title={row.title}
                        url={row.url}
                        windowLabel={row.windowLabel}
                      />
                      <div className="min-w-0 flex-1 text-left">
                        <button
                          className="min-w-0 w-full text-left"
                          data-testid={`browser-row-main-${row.key}`}
                          onClick={() => void openRow(row)}
                          type="button"
                        >
                          <div className="flex min-w-0 flex-wrap items-center gap-2">
                            <span className="truncate text-sm font-medium">
                              {row.title}
                            </span>
                            <BrowserRowColdChip
                              sid={row.mainSurfaceId}
                              windowLabel={row.windowLabel}
                            />
                            {row.host === "windowed" ? (
                              <Badge
                                className="normal-case tracking-normal"
                                variant="outline"
                              >
                                Window
                              </Badge>
                            ) : null}
                          </div>
                          <p className="mt-0.5 truncate text-2xs text-muted-foreground">
                            {browserRowUrlLabel(row.url)}
                          </p>
                          <p
                            className="mt-0.5 truncate text-2xs text-muted-foreground"
                            data-testid={`browser-row-viewport-${row.key}`}
                          >
                            {playgroundViewportCaption(
                              getPlaygroundViewport(row.mainSurfaceId),
                            )}
                          </p>
                        </button>
                        <BindingChips
                          bindings={bindings}
                          channelNames={channelNames}
                          onOpenChannel={(channelId) => {
                            void goChannel(channelId);
                          }}
                          onOpenThread={(channelId, threadRoot) => {
                            void goChannel(channelId, {
                              thread: threadRoot,
                              threadRootId: threadRoot,
                            });
                          }}
                        />
                      </div>
                      <div className="flex shrink-0 flex-wrap items-center justify-end gap-2">
                        {agentToggle}
                        <SiteRunbookOpenButton
                          label="Runbook"
                          runbookRef={sidRunbookRef(row.mainSurfaceId)}
                          testId={`browser-row-runbook-${row.key}`}
                        />
                        {browserRowShowsOpenButton(row) ? (
                          <Button
                            aria-label={
                              row.host === "windowed"
                                ? "Focus detached browser window"
                                : "Open browser"
                            }
                            data-testid={`browser-row-open-${row.key}`}
                            onClick={() => void openRow(row)}
                            size="xs"
                            type="button"
                            variant="secondary"
                          >
                            Open
                          </Button>
                        ) : null}
                        {row.host === "main" ? (
                          <Button
                            data-testid={`browser-row-detach-${row.key}`}
                            onClick={() => void detachRow(row)}
                            size="xs"
                            type="button"
                            variant="ghost"
                          >
                            <ExternalLink className="mr-1 h-3 w-3" />
                            Window
                          </Button>
                        ) : (
                          <span className="inline-flex items-center gap-1 text-2xs text-muted-foreground">
                            <AppWindow className="h-3 w-3" />
                            Detached
                          </span>
                        )}
                        {pendingRemoveKey === row.key ? (
                          <>
                            <Button
                              aria-label="Confirm remove browser"
                              data-testid={`browser-row-remove-confirm-${row.key}`}
                              onClick={() => {
                                setPendingRemoveKey(null);
                                void removeRow(row);
                              }}
                              size="xs"
                              title="Confirm remove"
                              type="button"
                              variant="destructive"
                            >
                              Confirm?
                            </Button>
                            <Button
                              aria-label="Cancel remove browser"
                              data-testid={`browser-row-remove-cancel-${row.key}`}
                              onClick={() => setPendingRemoveKey(null)}
                              size="xs"
                              title="Cancel"
                              type="button"
                              variant="ghost"
                            >
                              Cancel
                            </Button>
                          </>
                        ) : (
                          <Button
                            aria-label="Remove browser"
                            data-testid={`browser-row-remove-${row.key}`}
                            onClick={() => setPendingRemoveKey(row.key)}
                            size="xs"
                            title="Remove"
                            type="button"
                            variant="ghost"
                          >
                            <Trash2 className="h-3 w-3 text-destructive" />
                            <span className="ml-1 text-destructive">
                              Remove
                            </span>
                          </Button>
                        )}
                      </div>
                    </div>
                    {row.secondaryTabs.length > 0 ? (
                      <ul
                        className="ml-4 space-y-1 border-l border-border/60 pl-3"
                        data-testid={`browser-row-tabs-${row.key}`}
                      >
                        {row.secondaryTabs.map((tab) => (
                          <li key={tab.surfaceId}>
                            <button
                              className="flex w-full min-w-0 items-center gap-2 rounded-md px-1 py-1 text-left hover:bg-muted/50"
                              data-testid={`browser-row-tab-${tab.surfaceId}`}
                              onClick={() =>
                                void openSecondaryTab(row, tab.surfaceId)
                              }
                              type="button"
                            >
                              <BrowserRowPreview
                                key={`${tab.surfaceId}:${tab.url}`}
                                onOpen={() =>
                                  void openSecondaryTab(row, tab.surfaceId)
                                }
                                sid={tab.surfaceId}
                                title={tab.title}
                                url={tab.url}
                                windowLabel={row.windowLabel}
                              />
                              <span className="min-w-0 flex-1 truncate text-2xs text-muted-foreground">
                                {browserRowUrlLabel(tab.url)}
                              </span>
                            </button>
                          </li>
                        ))}
                      </ul>
                    ) : null}
                    </div>
                  )}
                </BrowserAgentChrome>
              </li>
            ))}
          </ul>
        )}
      </div>

      <AddBrowserDialog
        onOpenChange={setAddOpen}
        onSubmit={openAddedBrowser}
        open={addOpen}
      />
    </div>
  );
}
