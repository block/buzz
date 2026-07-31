import * as React from "react";

import type { McpAppMessage } from "@/features/mcp-apps/lib/mcpAppBridge";
import type { ChannelMcpAppInstallation } from "@/features/mcp-apps/lib/channelMcpAppStorage";
import { mcpAppDisplayText } from "@/features/mcp-apps/lib/mcpAppMessage";
import {
  McpAppFrame,
  type McpAppFrameProps,
} from "@/features/mcp-apps/ui/McpAppFrame";
import {
  connectMcpAppServer,
  disconnectMcpAppServer,
  type McpAppInvocationContext,
  type McpAppResourcePolicy,
} from "@/shared/api/tauriMcpApps";
import { channelChrome } from "@/shared/layout/chromeLayout";
import { cn } from "@/shared/lib/cn";

type ChannelMcpAppPaneProps = Pick<
  McpAppFrameProps,
  "onModelContext" | "onOpenLink"
> & {
  app: ChannelMcpAppInstallation;
  invocationContext: McpAppInvocationContext;
  header: React.ReactNode;
  onMessage?: (
    app: Pick<ChannelMcpAppInstallation, "id" | "title">,
    message: McpAppMessage,
  ) => Promise<void> | void;
};

export function ChannelMcpAppPane({
  app,
  invocationContext,
  header,
  onMessage,
  onModelContext,
  onOpenLink,
}: ChannelMcpAppPaneProps) {
  const [serverId, setServerId] = React.useState<string | null>(null);
  const [error, setError] = React.useState<string | null>(null);
  const serializedArguments = JSON.stringify(app.arguments);
  const serializedPolicy = JSON.stringify(app.approvedPolicy);
  const stableArguments = React.useMemo(
    () => JSON.parse(serializedArguments) as Record<string, unknown>,
    [serializedArguments],
  );
  const stablePolicy = React.useMemo(
    () => JSON.parse(serializedPolicy) as McpAppResourcePolicy,
    [serializedPolicy],
  );
  const messageApp = React.useMemo(
    () => ({ id: app.id, title: app.title }),
    [app.id, app.title],
  );
  const initialTool = React.useMemo(
    () => ({ name: app.toolName, arguments: stableArguments }),
    [app.toolName, stableArguments],
  );
  const handleMessage = React.useCallback(
    (message: McpAppMessage) => onMessage?.(messageApp, message),
    [messageApp, onMessage],
  );

  React.useEffect(() => {
    let active = true;
    let connectedServerId: string | null = null;
    setServerId(null);
    setError(null);
    void connectMcpAppServer(app.endpoint)
      .then((server) => {
        connectedServerId = server.serverId;
        if (!active) {
          void disconnectMcpAppServer(server.serverId);
          return;
        }
        const tool = server.tools.find(
          (candidate) =>
            candidate.name === app.toolName &&
            candidate.uiResourceUri === app.resourceUri,
        );
        if (!tool) {
          void disconnectMcpAppServer(server.serverId);
          connectedServerId = null;
          throw new Error(
            "The MCP server no longer advertises this app resource.",
          );
        }
        setServerId(server.serverId);
      })
      .catch((cause) => {
        if (!active) return;
        setError(
          mcpAppDisplayText(
            cause instanceof Error ? cause.message : "",
            "The MCP App is unavailable.",
          ),
        );
      });
    return () => {
      active = false;
      if (connectedServerId) {
        void disconnectMcpAppServer(connectedServerId);
      }
    };
  }, [app.endpoint, app.resourceUri, app.toolName]);

  return (
    <div className="relative flex min-h-0 min-w-0 flex-1 flex-col overflow-hidden">
      <div
        aria-hidden="true"
        className={cn(
          "pointer-events-none absolute inset-x-0 top-0 z-30 bg-background/80 backdrop-blur-md supports-backdrop-filter:bg-background/70 dark:bg-background/70 dark:backdrop-blur-xl dark:supports-backdrop-filter:bg-background/55",
          channelChrome.headerHeight,
        )}
      />
      {header}
      <section
        aria-label={`${app.title} channel app`}
        className={cn(
          "flex min-h-0 min-w-0 flex-1 flex-col overflow-hidden",
          channelChrome.contentPadding,
        )}
      >
        {serverId ? (
          <McpAppFrame
            approvedPolicy={stablePolicy}
            className="min-h-0 flex-1"
            initialTool={initialTool}
            invocationContext={invocationContext}
            key={app.id}
            onMessage={handleMessage}
            onModelContext={onModelContext}
            onOpenLink={onOpenLink}
            resourceUri={app.resourceUri}
            serverId={serverId}
            title={app.title}
          />
        ) : (
          <div className="grid min-h-0 flex-1 place-items-center p-8 text-center">
            <div className="max-w-sm space-y-1">
              <p className="text-sm font-medium text-foreground">
                {error ? "App unavailable" : `Opening ${app.title}…`}
              </p>
              {error ? (
                <p className="text-sm text-muted-foreground">{error}</p>
              ) : null}
            </div>
          </div>
        )}
      </section>
    </div>
  );
}
