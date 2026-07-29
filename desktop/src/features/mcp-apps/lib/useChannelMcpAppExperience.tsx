import * as React from "react";

import type { ChannelMcpAppInstallation } from "@/features/mcp-apps/lib/channelMcpAppStorage";
import {
  mcpAppAttributedMessage,
  MCP_APP_POST_MAX_CHARS,
  MCP_APP_POST_MAX_LINES,
  mcpAppMessageText,
} from "@/features/mcp-apps/lib/mcpAppMessage";
import { useChannelMcpApps } from "@/features/mcp-apps/lib/useChannelMcpApps";
import { ChannelMcpAppDialog } from "@/features/mcp-apps/ui/ChannelMcpAppDialog";
import { ChannelMcpAppPane } from "@/features/mcp-apps/ui/ChannelMcpAppPane";
import { ChannelMcpAppTabs } from "@/features/mcp-apps/ui/ChannelMcpAppTabs";
import type { Channel } from "@/shared/api/types";
import { Button } from "@/shared/ui/button";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/shared/ui/dialog";

type SendChannelMessage = (
  content: string,
  mentionPubkeys: string[],
  mediaTags?: string[][],
  channelId?: string | null,
) => Promise<void>;

type PendingChannelPost = {
  appId: string;
  appKey: string;
  appTitle: string;
  channelId: string;
  content: string;
  reject: (error: Error) => void;
  resolve: () => void;
};

const MCP_APP_POST_PROMPT_COOLDOWN_MS = 30_000;

export function pendingMcpAppPostInvalidationReason(
  pendingChannelId: string | null,
  channel: Pick<Channel, "archivedAt" | "id" | "isMember"> | null,
): string | null {
  if (!pendingChannelId) return null;
  if (pendingChannelId !== channel?.id) {
    return "The channel changed before the app post was approved.";
  }
  if (!channel.isMember || channel.archivedAt) {
    return "The channel became read-only before the app post was approved.";
  }
  return null;
}

export function pendingMcpAppRemovalReason(
  pendingAppId: string | null,
  installedAppIds: readonly string[],
): string | null {
  if (pendingAppId && !installedAppIds.includes(pendingAppId)) {
    return "The channel app was removed before the post was approved.";
  }
  return null;
}

export function useMcpAppUi(
  channel: Channel | null,
  pubkey: string | null | undefined,
  sendMessage: SendChannelMessage,
) {
  const [dialogOpen, setDialogOpen] = React.useState(false);
  const [pendingPost, setPendingPost] =
    React.useState<PendingChannelPost | null>(null);
  const pendingPostRef = React.useRef<PendingChannelPost | null>(null);
  const promptedAtRef = React.useRef(new Map<string, number>());
  const mutedAppKeysRef = React.useRef(new Set<string>());
  const [isPosting, setIsPosting] = React.useState(false);
  const [postError, setPostError] = React.useState<string | null>(null);
  const apps = useChannelMcpApps({
    channelId: channel?.id ?? null,
    pubkey,
  });
  const channelAppsAvailable =
    channel?.channelType !== "forum" &&
    channel?.isMember === true &&
    !channel.archivedAt;
  const activeApp = channelAppsAvailable ? apps.activeApp : null;
  const rejectPendingPost = React.useCallback((reason: string) => {
    const current = pendingPostRef.current;
    pendingPostRef.current = null;
    setPendingPost(null);
    setPostError(null);
    current?.reject(new Error(reason));
  }, []);
  React.useEffect(() => {
    const reason = pendingMcpAppPostInvalidationReason(
      pendingPostRef.current?.channelId ?? null,
      channel,
    );
    if (reason) {
      rejectPendingPost(reason);
    }
  }, [channel, rejectPendingPost]);
  React.useEffect(
    () => () =>
      rejectPendingPost("The channel app closed before the post was approved."),
    [rejectPendingPost],
  );
  React.useEffect(() => {
    const current = pendingPostRef.current;
    const reason = pendingMcpAppRemovalReason(
      current?.appId ?? null,
      apps.apps.map((installation) => installation.id),
    );
    if (reason) {
      rejectPendingPost(reason);
    }
  }, [apps.apps, rejectPendingPost]);
  React.useEffect(() => {
    if (!channelAppsAvailable) {
      setDialogOpen(false);
      if (apps.activeAppId !== null) {
        apps.showChat();
      }
    }
  }, [apps.activeAppId, apps.showChat, channelAppsAvailable]);
  const handleMessage = React.useCallback(
    async (
      app: ChannelMcpAppInstallation,
      message: Parameters<typeof mcpAppMessageText>[0],
    ) => {
      if (!channel?.isMember || channel.archivedAt || !channel.id) {
        throw new Error("This channel is read-only.");
      }
      const appKey = `${channel.id}:${app.id}`;
      if (mutedAppKeysRef.current.has(appKey)) {
        throw new Error("Channel post requests from this app are muted.");
      }
      const now = Date.now();
      const promptedAt = promptedAtRef.current.get(appKey) ?? 0;
      if (now - promptedAt < MCP_APP_POST_PROMPT_COOLDOWN_MS) {
        throw new Error("This app requested another channel post too quickly.");
      }
      const content = mcpAppMessageText(message);
      if (!content) {
        throw new Error("The app message did not contain text.");
      }
      if (content.length > MCP_APP_POST_MAX_CHARS) {
        throw new Error(
          `The app message exceeds the ${MCP_APP_POST_MAX_CHARS.toLocaleString()} character limit.`,
        );
      }
      if (content.split("\n").length > MCP_APP_POST_MAX_LINES) {
        throw new Error(
          `The app message exceeds the ${MCP_APP_POST_MAX_LINES.toLocaleString()} line limit.`,
        );
      }
      if (pendingPostRef.current) {
        throw new Error("Another app post is waiting for approval.");
      }
      promptedAtRef.current.set(appKey, now);
      await new Promise<void>((resolve, reject) => {
        const next = {
          appId: app.id,
          appKey,
          appTitle: app.title,
          channelId: channel.id,
          content,
          reject,
          resolve,
        };
        pendingPostRef.current = next;
        setPostError(null);
        setPendingPost(next);
      });
    },
    [channel?.archivedAt, channel?.id, channel?.isMember],
  );
  const handleMuteAppPosts = React.useCallback(() => {
    const current = pendingPostRef.current;
    if (!current) return;
    mutedAppKeysRef.current.add(current.appKey);
    rejectPendingPost("Channel post requests from this app are muted.");
  }, [rejectPendingPost]);
  const handleApprovePost = React.useCallback(async () => {
    const current = pendingPostRef.current;
    if (!current || !channel) return;
    const invalidationReason = pendingMcpAppPostInvalidationReason(
      current.channelId,
      channel,
    );
    if (invalidationReason) {
      rejectPendingPost(invalidationReason);
      return;
    }
    const removalReason = pendingMcpAppRemovalReason(
      current.appId,
      apps.apps.map((installation) => installation.id),
    );
    if (removalReason) {
      rejectPendingPost(removalReason);
      return;
    }
    setIsPosting(true);
    setPostError(null);
    try {
      await sendMessage(
        mcpAppAttributedMessage(current.appTitle, current.content),
        [],
        undefined,
        channel.id,
      );
      if (pendingPostRef.current === current) {
        pendingPostRef.current = null;
        setPendingPost(null);
        current.resolve();
      }
    } catch (cause) {
      setPostError(
        cause instanceof Error
          ? cause.message
          : "Buzz could not post the app message.",
      );
    } finally {
      setIsPosting(false);
    }
  }, [apps.apps, channel, rejectPendingPost, sendMessage]);
  const approvedPostPreview = pendingPost
    ? mcpAppAttributedMessage(pendingPost.appTitle, pendingPost.content)
    : "";
  const navigation = React.useMemo(
    () =>
      channelAppsAvailable && channel && pubkey ? (
        <>
          <ChannelMcpAppTabs
            activeAppId={apps.activeAppId}
            apps={apps.apps}
            onActivateApp={apps.activateApp}
            onOpenApps={() => setDialogOpen(true)}
            onShowChat={apps.showChat}
          />
          <ChannelMcpAppDialog
            apps={apps.apps}
            channelId={channel.id}
            onOpenChange={setDialogOpen}
            open={dialogOpen}
            pubkey={pubkey}
          />
          <Dialog
            modal={false}
            onOpenChange={(open) => {
              if (!open && !isPosting) {
                rejectPendingPost("The channel post was not approved.");
              }
            }}
            open={pendingPost !== null}
          >
            <DialogContent
              className="max-w-lg"
              overlayClassName="pointer-events-none"
            >
              <DialogHeader>
                <DialogTitle>Post requested by a channel app?</DialogTitle>
                <DialogDescription>
                  Review the exact message requested by “
                  {pendingPost?.appTitle ?? "Channel app"}” before Buzz posts it
                  in #{channel.name}.
                </DialogDescription>
              </DialogHeader>
              <pre className="max-h-64 overflow-auto whitespace-pre-wrap rounded-xl border border-border/60 bg-muted/30 p-3 font-sans text-sm text-foreground">
                {approvedPostPreview}
              </pre>
              {postError ? (
                <p className="text-sm text-destructive" role="alert">
                  {postError}
                </p>
              ) : null}
              <DialogFooter>
                <Button
                  disabled={isPosting}
                  onClick={() =>
                    rejectPendingPost("The channel post was not approved.")
                  }
                  type="button"
                  variant="ghost"
                >
                  Cancel
                </Button>
                <Button
                  disabled={isPosting}
                  onClick={handleMuteAppPosts}
                  type="button"
                  variant="outline"
                >
                  Mute for now
                </Button>
                <Button
                  disabled={isPosting}
                  onClick={() => void handleApprovePost()}
                  type="button"
                >
                  {isPosting ? "Posting…" : "Post to channel"}
                </Button>
              </DialogFooter>
            </DialogContent>
          </Dialog>
        </>
      ) : undefined,
    [
      approvedPostPreview,
      apps.activateApp,
      apps.activeAppId,
      apps.apps,
      apps.showChat,
      channel,
      channelAppsAvailable,
      dialogOpen,
      handleApprovePost,
      handleMuteAppPosts,
      isPosting,
      pendingPost,
      postError,
      pubkey,
      rejectPendingPost,
    ],
  );
  const renderPane = React.useCallback(
    (header: React.ReactNode) =>
      activeApp ? (
        <ChannelMcpAppPane
          app={activeApp}
          header={header}
          onMessage={handleMessage}
        />
      ) : null,
    [activeApp, handleMessage],
  );

  return {
    active: activeApp !== null,
    navigation,
    renderPane,
  };
}
