import * as React from "react";

import {
  callMcpAppTool,
  prepareMcpAppView,
  releaseMcpAppView,
  type McpAppInvocationContext,
  type McpAppResourcePermissions,
  type McpAppResourcePolicy,
} from "@/shared/api/tauriMcpApps";
import {
  connectMcpAppBridge,
  createMcpAppBridge,
  mcpAppSandboxOrigin,
  observeMcpAppHostContext,
  type McpAppBridgeCallbacks,
} from "@/features/mcp-apps/lib/mcpAppBridge";
import { mcpAppDisplayText } from "@/features/mcp-apps/lib/mcpAppMessage";

const PROXY_READY_METHOD = "ui/notifications/sandbox-proxy-ready";

type InitialToolBridge = Pick<
  ReturnType<typeof createMcpAppBridge>,
  "sendToolCancelled" | "sendToolInput" | "sendToolResult"
>;

export type InitialToolLifecycle = {
  started: boolean;
  terminalSent: boolean;
};

export async function runInitialMcpAppTool(
  bridge: InitialToolBridge,
  serverId: string,
  initialTool: NonNullable<McpAppFrameProps["initialTool"]>,
  lifecycle: InitialToolLifecycle,
  callTool: typeof callMcpAppTool = callMcpAppTool,
): Promise<void> {
  lifecycle.started = true;
  try {
    await bridge.sendToolInput({ arguments: initialTool.arguments });
    const result = await callTool(
      serverId,
      initialTool.name,
      initialTool.arguments,
      "host",
    );
    if (!lifecycle.terminalSent) {
      lifecycle.terminalSent = true;
      await bridge.sendToolResult(result);
    }
  } catch (cause) {
    if (!lifecycle.terminalSent) {
      lifecycle.terminalSent = true;
      await bridge
        .sendToolCancelled({ reason: "The initial MCP App tool call failed." })
        .catch(() => undefined);
    }
    throw cause;
  }
}

export type McpAppFrameProps = McpAppBridgeCallbacks & {
  serverId: string;
  resourceUri: string;
  approvedPolicy: McpAppResourcePolicy;
  title: string;
  invocationContext?: McpAppInvocationContext;
  initialTool?: {
    name: string;
    arguments: Record<string, unknown>;
  };
  className?: string;
  onReady?: () => void;
  onError?: (error: Error) => void;
  onPermissionsRequested?: (permissions: McpAppResourcePermissions) => void;
};

function waitForSandboxProxy(
  iframe: HTMLIFrameElement,
  sandboxUrl: string,
  expectedOrigin: string,
  signal: AbortSignal,
): Promise<void> {
  return new Promise((resolve, reject) => {
    const timeout = window.setTimeout(() => {
      cleanup();
      reject(new Error("MCP App sandbox did not become ready"));
    }, 10_000);
    const cleanup = () => {
      window.clearTimeout(timeout);
      window.removeEventListener("message", onMessage);
      signal.removeEventListener("abort", onAbort);
    };
    const onAbort = () => {
      cleanup();
      reject(new DOMException("MCP App load cancelled", "AbortError"));
    };
    const onMessage = (event: MessageEvent) => {
      if (
        event.source === iframe.contentWindow &&
        event.origin === expectedOrigin &&
        event.data?.method === PROXY_READY_METHOD
      ) {
        cleanup();
        resolve();
      }
    };
    window.addEventListener("message", onMessage);
    signal.addEventListener("abort", onAbort, { once: true });
    iframe.src = sandboxUrl;
  });
}

/**
 * Standards-compliant MCP App frame. The iframe loads only Buzz's trusted
 * outer proxy; untrusted resource HTML is inserted into its inner sandbox.
 */
export function McpAppFrame({
  serverId,
  resourceUri,
  approvedPolicy,
  title,
  invocationContext,
  initialTool,
  className,
  onReady,
  onError,
  onPermissionsRequested,
  onMessage,
  onModelContext,
  onOpenLink,
  onSizeChange,
}: McpAppFrameProps) {
  const iframeRef = React.useRef<HTMLIFrameElement>(null);
  const [error, setError] = React.useState<string | null>(null);

  React.useEffect(() => {
    const iframe = iframeRef.current;
    if (!iframe) return;

    const abortController = new AbortController();
    let bridge: ReturnType<typeof createMcpAppBridge> | null = null;
    let disposeContext: (() => void) | null = null;
    let viewId: string | null = null;
    const initialToolLifecycle: InitialToolLifecycle = {
      started: false,
      terminalSent: false,
    };

    const start = async () => {
      try {
        setError(null);
        const prepared = await prepareMcpAppView(
          serverId,
          resourceUri,
          approvedPolicy,
        );
        if (abortController.signal.aborted) {
          await releaseMcpAppView(prepared.viewId).catch(() => undefined);
          return;
        }
        viewId = prepared.viewId;
        onPermissionsRequested?.(prepared.requestedPermissions);
        const sandboxOrigin = mcpAppSandboxOrigin(prepared.sandboxUrl);

        bridge = createMcpAppBridge(
          serverId,
          {
            onMessage,
            onModelContext,
            onOpenLink,
            onSizeChange,
          },
          invocationContext,
        );
        await waitForSandboxProxy(
          iframe,
          prepared.sandboxUrl,
          sandboxOrigin,
          abortController.signal,
        );
        await connectMcpAppBridge(bridge, iframe, sandboxOrigin);

        const initialized = new Promise<void>((resolve) => {
          const handleInitialized = () => {
            bridge?.removeEventListener("initialized", handleInitialized);
            resolve();
          };
          bridge?.addEventListener("initialized", handleInitialized);
        });
        await bridge.sendSandboxResourceReady({
          html: prepared.html,
          csp: prepared.csp,
          permissions: {},
        });
        await initialized;
        if (abortController.signal.aborted) return;

        disposeContext = observeMcpAppHostContext(bridge, iframe);
        if (initialTool) {
          await runInitialMcpAppTool(
            bridge,
            serverId,
            initialTool,
            initialToolLifecycle,
            (serverId, name, argumentsValue, caller) =>
              callMcpAppTool(
                serverId,
                name,
                argumentsValue,
                caller,
                invocationContext,
              ),
          );
        }
        if (abortController.signal.aborted) return;
        onReady?.();
      } catch (cause) {
        if (abortController.signal.aborted) return;
        const next =
          cause instanceof Error ? cause : new Error("MCP App failed to load");
        setError(mcpAppDisplayText(next.message, "MCP App failed to load"));
        onError?.(next);
      }
    };

    void start();
    return () => {
      abortController.abort();
      disposeContext?.();
      const closingBridge = bridge;
      const closingViewId = viewId;
      void (async () => {
        if (closingBridge) {
          if (
            initialToolLifecycle.started &&
            !initialToolLifecycle.terminalSent
          ) {
            initialToolLifecycle.terminalSent = true;
            await closingBridge
              .sendToolCancelled({ reason: "The channel app was closed." })
              .catch(() => undefined);
          }
          await closingBridge.teardownResource({}).catch(() => undefined);
          await closingBridge.close().catch(() => undefined);
        }
        if (closingViewId) {
          await releaseMcpAppView(closingViewId).catch(() => undefined);
        }
      })();
    };
  }, [
    initialTool,
    approvedPolicy,
    onError,
    onMessage,
    onModelContext,
    onOpenLink,
    onPermissionsRequested,
    onReady,
    onSizeChange,
    resourceUri,
    serverId,
    invocationContext,
  ]);

  return (
    <div
      className={`relative overflow-hidden ${className ?? ""}`}
      data-testid="mcp-app-frame"
    >
      <iframe
        className="h-full min-h-48 w-full border-0 bg-background"
        ref={iframeRef}
        sandbox="allow-same-origin allow-scripts"
        title={title}
      />
      {error ? (
        <div
          className="absolute inset-0 grid place-items-center bg-background/95 p-6 text-center text-sm text-muted-foreground"
          role="alert"
        >
          {error}
        </div>
      ) : null}
    </div>
  );
}
