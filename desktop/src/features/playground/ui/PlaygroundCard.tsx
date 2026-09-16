import { isTauri } from "@tauri-apps/api/core";
import { useLocation } from "@tanstack/react-router";
import { AppWindow, Copy } from "lucide-react";
import * as React from "react";
import { toast } from "sonner";

import { deriveShellRoute } from "@/app/AppShell.helpers";
import {
  openPopoutWindow,
  popoutErrorMessage,
} from "@/features/popout/lib/popoutWindow";
import {
  playgroundConversationFromRoute,
  playgroundConversationHasOpenThread,
} from "@/features/playground/lib/conversation";
import { copyTextToClipboard } from "@/shared/lib/clipboard";
import { Button } from "@/shared/ui/button";
import {
  Attachment,
  AttachmentActions,
  AttachmentContent,
  AttachmentMedia,
  AttachmentTitle,
} from "@/shared/ui/attachment";

import { probePlaygroundUrl } from "../lib/probe";
import {
  addPlaygroundSession,
  hasPlaygroundSession,
  notePlaygroundCard,
  showPlaygroundSession,
} from "../lib/sessions";
import {
  playgroundPin,
  type PlaygroundCard as PlaygroundCardData,
} from "../lib/types";

function canHostPlayground(): boolean {
  return (
    isTauri() ||
    import.meta.env.MODE === "e2e" ||
    import.meta.env.MODE === "test"
  );
}

async function openPlaygroundInBrowser(url: string) {
  if (import.meta.env.MODE === "test") {
    const stub = (
      globalThis as { __BUZZ_PLAYGROUND_OPEN_URL__?: (nextUrl: string) => void }
    ).__BUZZ_PLAYGROUND_OPEN_URL__;
    if (stub) {
      stub(url);
      return;
    }
  }
  try {
    const { openUrl } = await import("@tauri-apps/plugin-opener");
    await openUrl(url);
    return;
  } catch {
    window.open(url, "_blank", "noopener,noreferrer");
  }
}

export function PlaygroundCard({ card }: { card: PlaygroundCardData }) {
  const [busy, setBusy] = React.useState(false);
  const host = canHostPlayground();
  const pin = playgroundPin(card);
  const location = useLocation();
  const conversation = React.useMemo(() => {
    const route = deriveShellRoute(location.pathname);
    const search = location.search as {
      thread?: unknown;
      threadRootId?: unknown;
    };
    const thread = search.threadRootId ?? search.thread;
    return playgroundConversationFromRoute({
      selectedView: route.selectedView,
      selectedChannelId: route.selectedChannelId,
      threadId: typeof thread === "string" ? thread : null,
    });
  }, [location.pathname, location.search]);

  React.useEffect(() => {
    notePlaygroundCard(card);
  }, [card]);

  async function ensureUp(): Promise<boolean> {
    const result = await probePlaygroundUrl(card.url);
    if (!result.up) {
      toast.error(result.message ?? "Playground is down.");
      return false;
    }
    return true;
  }

  function handlePin() {
    if (!host) {
      void openPlaygroundInBrowser(card.url);
      return;
    }
    if (hasPlaygroundSession(card.sid)) {
      showPlaygroundSession(card.sid);
      return;
    }
    setBusy(true);
    void (async () => {
      try {
        if (!(await ensureUp())) return;
        addPlaygroundSession(card);
      } catch (error) {
        toast.error(popoutErrorMessage(error, "Playground is down."));
      } finally {
        setBusy(false);
      }
    })();
  }

  function handleOpen() {
    if (!host) {
      void openPlaygroundInBrowser(card.url);
      return;
    }
    setBusy(true);
    void (async () => {
      try {
        if (!(await ensureUp())) return;
        await openPopoutWindow({
          kind: "playground",
          title: card.name,
          seed: card.sid,
          playground: card,
        });
      } catch (error) {
        toast.error(popoutErrorMessage(error, "Could not open playground."));
      } finally {
        setBusy(false);
      }
    })();
  }

  const isThreadConversation =
    playgroundConversationHasOpenThread(conversation);

  function handleOpenAsSplit() {
    const threadId = conversation?.draftKey.startsWith("thread:")
      ? conversation.draftKey.slice("thread:".length)
      : undefined;
    if (!conversation || !threadId) {
      toast.error("Open a thread first.");
      return;
    }
    if (!host) {
      void openPlaygroundInBrowser(card.url);
      return;
    }
    setBusy(true);
    void (async () => {
      try {
        if (!(await ensureUp())) return;
        await openPopoutWindow({
          kind: "split",
          title: card.name,
          seed: `${card.sid}-${conversation.channelId}-${threadId}`,
          channelId: conversation.channelId,
          threadId,
          playground: card,
        });
      } catch (error) {
        toast.error(popoutErrorMessage(error, "Could not open split."));
      } finally {
        setBusy(false);
      }
    })();
  }

  function handleUrlClick(event: React.MouseEvent<HTMLAnchorElement>) {
    event.preventDefault();
    event.stopPropagation();
    void openPlaygroundInBrowser(card.url);
  }

  function handleCopyPin(event: React.MouseEvent<HTMLButtonElement>) {
    event.preventDefault();
    event.stopPropagation();
    if (!pin) return;
    copyTextToClipboard(pin, "PIN copied");
  }

  return (
    <Attachment
      className="my-2 max-w-md items-start"
      data-testid="playground-card"
    >
      <AttachmentMedia>
        <AppWindow />
      </AttachmentMedia>
      <div className="flex min-w-0 flex-1 flex-col gap-2">
        <AttachmentContent>
          <AttachmentTitle data-testid="playground-card-name">
            {card.name}
          </AttachmentTitle>
          <a
            className="block truncate text-xs leading-4 text-muted-foreground hover:text-foreground hover:underline"
            data-testid="playground-card-url"
            href={card.url}
            onClick={handleUrlClick}
            rel="noopener noreferrer"
          >
            {card.url}
          </a>
          {pin || card.stack ? (
            <p className="mt-1 flex items-center gap-1 text-xs text-muted-foreground">
              {pin ? (
                <>
                  PIN{" "}
                  <span
                    className="font-mono text-foreground"
                    data-testid="playground-card-pin"
                  >
                    {pin}
                  </span>
                  <button
                    aria-label="Copy PIN"
                    className="inline-flex h-5 w-5 items-center justify-center rounded-sm text-muted-foreground hover:bg-muted hover:text-foreground"
                    data-testid="playground-card-copy-pin"
                    onClick={handleCopyPin}
                    type="button"
                  >
                    <Copy className="h-3 w-3" />
                  </button>
                </>
              ) : null}
              {card.stack ? (
                <>
                  {pin ? " " : null}·{" "}
                  <span data-testid="playground-card-stack">{card.stack}</span>
                </>
              ) : null}
            </p>
          ) : null}
        </AttachmentContent>
        <AttachmentActions className="w-full flex-wrap justify-end">
          <Button
            data-testid="playground-card-pin-action"
            disabled={busy}
            onClick={(event) => {
              event.stopPropagation();
              handlePin();
            }}
            size="sm"
            type="button"
            variant="outline"
          >
            Pin
          </Button>
          <Button
            data-testid="playground-card-open"
            disabled={busy}
            onClick={(event) => {
              event.stopPropagation();
              handleOpen();
            }}
            size="sm"
            type="button"
          >
            Open
          </Button>
          <Button
            data-testid="playground-card-open-split"
            disabled={busy || !isThreadConversation}
            onClick={(event) => {
              event.stopPropagation();
              handleOpenAsSplit();
            }}
            size="sm"
            type="button"
            variant="outline"
          >
            Open as Split
          </Button>
        </AttachmentActions>
      </div>
    </Attachment>
  );
}
