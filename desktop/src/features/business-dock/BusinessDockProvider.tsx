import * as React from "react";
import { openUrl } from "@tauri-apps/plugin-opener";
import { toast } from "sonner";

import {
  createBusinessBridgeMessage,
  createBusinessBridgeV2Message,
  createBusinessBridgeV3Message,
  createBusinessSessionNonce,
  readBusinessBridgeEvent,
} from "@/features/business-dock/businessDockBridge";
import {
  buildBusinessEmbedBootstrapUrl,
  buildBusinessEmbedLoginUrl,
  businessSsoMode,
  canAttemptBusinessRecovery,
  type EmbedSessionPhase,
  subscribeToBusinessEmbedCallbacks,
} from "@/features/business-dock/businessEmbedSession";
import { startBusinessAuthHeartbeat } from "@/features/business-dock/businessAuthHeartbeat";
import {
  type BusinessDockConfig,
  getBusinessDockConfig,
} from "@/features/business-dock/businessDockConfig";
import {
  DEFAULT_BUSINESS_DOCK_PREFERENCES,
  readBusinessDockPreferences,
  saveBusinessDockPreferences,
} from "@/features/business-dock/businessDockPreferences";
import {
  canNavigateBusinessResource,
  keepLatestBusinessNavigation,
  shouldQueuePendingBusinessNavigation,
} from "@/features/business-dock/businessDockProviderPolicy";
import {
  canMoveBusinessNavigation,
  createBusinessNavigationState,
  currentBusinessNavigationEntry,
  moveBusinessNavigation,
  pushBusinessNavigation,
  type BusinessNavigationState,
  updateCurrentBusinessNavigation,
} from "@/features/business-dock/businessNavigation";
import {
  buildBusinessUrl,
  type BusinessResource,
  parseBusinessUrl,
  resolveBusinessResource,
} from "@/features/business-dock/businessResourceResolver";
import {
  businessDockReducer,
  createInitialBusinessDockState,
  type BusinessDockState,
} from "@/features/business-dock/businessDockStore";
import { useBusinessDockWidth } from "@/features/business-dock/useBusinessDockWidth";
import { useWorkbenchAuth } from "@/features/workbench-auth";
import { issueEmbedSession } from "@/features/workbench-auth/businessAuthGateway";
import { useTheme } from "@/shared/theme/ThemeProvider";
import {
  AlertDialog,
  AlertDialogAction,
  AlertDialogCancel,
  AlertDialogContent,
  AlertDialogDescription,
  AlertDialogFooter,
  AlertDialogHeader,
  AlertDialogTitle,
} from "@/shared/ui/alert-dialog";

export const BUSINESS_DATA_CHANGED_EVENT = "buzz:business-data-changed";

type BridgeVersion = 1 | 2 | null;
type NavigationMode = "push" | "replace" | "history";
export type BusinessAuthState = {
  phase:
    | "unconnected"
    | "checking"
    | "signing-in"
    | "authenticated"
    | "expired"
    | "failed";
  identity?: { subject: string; displayName: string };
  reason?: string;
};

type BusinessDockContextValue = {
  bridgeReady: boolean;
  businessAuth: BusinessAuthState;
  embedSessionPhase: EmbedSessionPhase;
  ssoMode: ReturnType<typeof businessSsoMode>;
  workbenchAuthPhase: ReturnType<typeof useWorkbenchAuth>["phase"];
  workbenchGatewayStatus: ReturnType<
    typeof useWorkbenchAuth
  >["gatewayState"]["status"];
  workbenchGroupClaimStatus: ReturnType<
    typeof useWorkbenchAuth
  >["groupClaimStatus"];
  canGoBack: boolean;
  canGoForward: boolean;
  config: BusinessDockConfig | null;
  configError: string | null;
  iframeRef: React.RefObject<HTMLIFrameElement | null>;
  isOverlay: boolean;
  onBrowserLoad: () => void;
  onResetWidth: () => void;
  onResizeStart: React.MouseEventHandler<HTMLButtonElement>;
  renderedWidthPx: number;
  state: BusinessDockState;
  canResetWidth: boolean;
  close: () => void;
  getCurrentBusinessResource: () => BusinessResource | null;
  goBack: () => void;
  goForward: () => void;
  goHome: () => void;
  navigateBusinessResource: (resource: BusinessResource) => void;
  openBusinessResource: (resource: BusinessResource) => void;
  openBusinessResourceInBrowser: (resource: BusinessResource) => void;
  openBusinessResourceLink: (value: string) => boolean;
  openCurrentInBrowser: () => void;
  refresh: () => void;
  checkBusinessAuth: () => void;
  logoutBusiness: () => void;
  startBusinessSignIn: () => void;
  bindCurrentDevice: () => void;
  requestCurrentBusinessResource: () => void;
  resolveBusinessResourceLink: (value: string) => BusinessResource | null;
  setBusinessContext: (context: {
    legalEntityId?: string;
    period?: string;
  }) => void;
  toggle: () => void;
  toggleFollowConversation: () => void;
  toggleFullscreen: () => void;
  togglePinned: () => void;
};

const BusinessDockContext =
  React.createContext<BusinessDockContextValue | null>(null);

function debugBusinessDock(message: string): void {
  if (import.meta.env.DEV) console.debug(`[Business Dock] ${message}`);
}

function getBusinessDockLocalStorage(): Storage | null {
  try {
    return window.localStorage;
  } catch {
    return null;
  }
}

export function BusinessDockProvider({
  children,
}: {
  children: React.ReactNode;
}) {
  const configResult = React.useMemo(getBusinessDockConfig, []);
  const config = configResult.config;
  const preferences = React.useMemo(
    () =>
      typeof window === "undefined"
        ? DEFAULT_BUSINESS_DOCK_PREFERENCES
        : readBusinessDockPreferences(getBusinessDockLocalStorage()),
    [],
  );
  const [state, dispatch] = React.useReducer(
    businessDockReducer,
    undefined,
    () => createInitialBusinessDockState(config, preferences),
  );
  const initialResource = React.useMemo(
    () => (config ? parseBusinessUrl(config.homeUrl, config) : null),
    [config],
  );
  const [navigation, setNavigation] = React.useState<BusinessNavigationState>(
    () => createBusinessNavigationState(initialResource),
  );
  const navigationRef = React.useRef(navigation);
  const stateRef = React.useRef(state);
  const iframeRef = React.useRef<HTMLIFrameElement>(null);
  const sessionNonceRef = React.useRef(createBusinessSessionNonce());
  const pendingNavigationRef = React.useRef<BusinessResource | null>(null);
  const pendingLeaveRef = React.useRef<(() => void) | null>(null);
  const [leaveConfirmationOpen, setLeaveConfirmationOpen] =
    React.useState(false);
  const [bridgeVersion, setBridgeVersion] = React.useState<BridgeVersion>(null);
  const [businessAuth, setBusinessAuth] = React.useState<BusinessAuthState>({
    phase: "unconnected",
  });
  const [embedSessionPhase, setEmbedSessionPhase] =
    React.useState<EmbedSessionPhase>("idle");
  const ssoMode = React.useMemo(businessSsoMode, []);
  const workbenchAuth = useWorkbenchAuth();
  const recoveryAttemptsRef = React.useRef(0);
  const popupPollRef = React.useRef<ReturnType<
    typeof window.setInterval
  > | null>(null);
  const embedTimeoutRef = React.useRef<ReturnType<
    typeof window.setTimeout
  > | null>(null);
  const { isDark } = useTheme();
  const width = useBusinessDockWidth();

  React.useEffect(() => {
    stateRef.current = state;
  }, [state]);
  React.useEffect(() => {
    navigationRef.current = navigation;
  }, [navigation]);
  React.useEffect(() => {
    saveBusinessDockPreferences(getBusinessDockLocalStorage(), {
      followConversation: state.followConversation,
      pinned: state.pinned,
    });
  }, [state.followConversation, state.pinned]);

  React.useEffect(
    () => () => {
      if (popupPollRef.current) window.clearInterval(popupPollRef.current);
      if (embedTimeoutRef.current) window.clearTimeout(embedTimeoutRef.current);
    },
    [],
  );

  const postV1 = React.useCallback(
    (
      type: "HOST_INIT" | "SET_THEME" | "REFRESH" | "NAVIGATE",
      payload?: unknown,
    ) => {
      if (!config || !iframeRef.current?.contentWindow) return false;
      iframeRef.current.contentWindow.postMessage(
        createBusinessBridgeMessage(type, payload),
        config.origin,
      );
      return true;
    },
    [config],
  );
  const postV2 = React.useCallback(
    (
      type: Parameters<typeof createBusinessBridgeV2Message>[0],
      payload?: unknown,
    ) => {
      if (!config || !iframeRef.current?.contentWindow) return false;
      iframeRef.current.contentWindow.postMessage(
        createBusinessBridgeV2Message(type, sessionNonceRef.current, payload),
        config.origin,
      );
      return true;
    },
    [config],
  );
  const postV3 = React.useCallback(
    (
      type: Parameters<typeof createBusinessBridgeV3Message>[0],
      payload?: unknown,
    ) => {
      if (!config || !iframeRef.current?.contentWindow) return false;
      iframeRef.current.contentWindow.postMessage(
        createBusinessBridgeV3Message(type, sessionNonceRef.current, payload),
        config.origin,
      );
      return true;
    },
    [config],
  );

  React.useEffect(
    () =>
      startBusinessAuthHeartbeat(
        state.open && businessAuth.phase === "authenticated",
        () => postV3("CHECK_AUTH", { interactive: false }),
      ),
    [businessAuth.phase, postV3, state.open],
  );

  const previousWorkbenchAuthPhaseRef = React.useRef(workbenchAuth.phase);
  React.useEffect(() => {
    const previous = previousWorkbenchAuthPhaseRef.current;
    previousWorkbenchAuthPhaseRef.current = workbenchAuth.phase;
    if (previous !== "authenticated" || workbenchAuth.phase === "authenticated")
      return;
    postV3("LOGOUT");
    recoveryAttemptsRef.current = 0;
    setEmbedSessionPhase("idle");
    setBusinessAuth({ phase: "unconnected" });
  }, [postV3, workbenchAuth.phase]);

  const sendNavigation = React.useCallback(
    (resource: BusinessResource) => {
      if (!config) return false;
      const url = buildBusinessUrl(resource, config);
      if (!url) return false;
      if (bridgeVersion === 2) return postV2("NAVIGATE", { resource });
      if (bridgeVersion === 1) return postV1("NAVIGATE", { url });
      if (iframeRef.current) {
        iframeRef.current.src = url;
        return true;
      }
      return false;
    },
    [bridgeVersion, config, postV1, postV2],
  );

  const commitNavigation = React.useCallback(
    (resource: BusinessResource, mode: NavigationMode = "push") => {
      if (!config) return;
      const normalized = resolveBusinessResource(resource, config);
      if (!normalized) return;
      const url = buildBusinessUrl(normalized, config);
      if (!url) return;
      if (mode === "push") {
        setNavigation((current) => pushBusinessNavigation(current, normalized));
      } else if (mode === "replace") {
        setNavigation((current) =>
          updateCurrentBusinessNavigation(current, normalized),
        );
      }
      dispatch({ type: "open" });
      dispatch({ type: "dirty", dirty: false });
      dispatch({
        type: "navigate",
        url,
        resource: normalized,
        openingResource: true,
      });
      pendingNavigationRef.current = shouldQueuePendingBusinessNavigation(
        bridgeVersion,
      )
        ? keepLatestBusinessNavigation(pendingNavigationRef.current, normalized)
        : null;
      sendNavigation(normalized);
      debugBusinessDock("Business Resource opened");
    },
    [bridgeVersion, config, sendNavigation],
  );

  const requestLeave = React.useCallback((action: () => void) => {
    if (!stateRef.current.dirty) {
      action();
      return;
    }
    pendingLeaveRef.current = action;
    setLeaveConfirmationOpen(true);
  }, []);

  const openBusinessResource = React.useCallback(
    (resource: BusinessResource) => {
      if (!config) return;
      const normalized = resolveBusinessResource(resource, config);
      if (!normalized) return;
      if (
        !canNavigateBusinessResource({
          followConversation: stateRef.current.followConversation,
          pinned: stateRef.current.pinned,
          source: "explicit",
        })
      )
        return;
      if (stateRef.current.currentResource?.path === normalized.path) {
        dispatch({ type: "open" });
        return;
      }
      requestLeave(() => commitNavigation(normalized, "push"));
    },
    [commitNavigation, config, requestLeave],
  );

  const beginBusinessSignIn = React.useCallback(
    (automatic = false) => {
      if (!config) return;
      if (workbenchAuth.phase !== "authenticated") {
        setBusinessAuth({
          phase: "failed",
          reason: "Workbench authentication must be renewed first.",
        });
        setEmbedSessionPhase("failed");
        return;
      }
      const resource = stateRef.current.currentResource;
      const targetUrl = resource
        ? buildBusinessUrl(resource, config)
        : stateRef.current.currentUrl || config.homeUrl;
      if (!targetUrl) {
        setBusinessAuth({
          phase: "failed",
          reason: "Business URL is unavailable.",
        });
        setEmbedSessionPhase("failed");
        return;
      }

      setBusinessAuth({ phase: "signing-in" });
      if (ssoMode === "desktop-embed-session") {
        const gatewayUrl = workbenchAuth.gatewayUrl;
        if (gatewayUrl) {
          if (workbenchAuth.gatewayState.status !== "authenticated") {
            setBusinessAuth({
              phase: "failed",
              reason:
                workbenchAuth.gatewayState.status === "device_revoked"
                  ? "This Business Dock device binding was revoked."
                  : "Bind the current Buzz identity and device before opening Business Dock.",
            });
            setEmbedSessionPhase("failed");
            return;
          }
          setEmbedSessionPhase("authorizing");
          void (async () => {
            const accessToken = await workbenchAuth.getAccessToken();
            if (!accessToken) throw new Error("Workbench session expired.");
            const normalized = resource ?? {
              version: 1 as const,
              type: "generic" as const,
              id: "home",
              path: "/embed/",
            };
            const issued = await issueEmbedSession(gatewayUrl, accessToken, {
              type: normalized.type,
              id: normalized.id ?? "collection",
              path: normalized.path.startsWith("/embed/")
                ? normalized.path
                : "/embed/",
            });
            if (!iframeRef.current)
              throw new Error("Business iframe is unavailable.");
            setBusinessAuth({ phase: "checking" });
            setEmbedSessionPhase("redeeming");
            iframeRef.current.src = issued.embedUrl;
          })().catch((cause) => {
            setBusinessAuth({
              phase: "failed",
              reason:
                cause instanceof Error
                  ? cause.message
                  : "Embed Session issue failed.",
            });
            setEmbedSessionPhase("failed");
          });
          return;
        }
        const loginUrl = buildBusinessEmbedLoginUrl(config, targetUrl);
        if (!loginUrl) {
          setBusinessAuth({
            phase: "failed",
            reason: "Business target was rejected.",
          });
          setEmbedSessionPhase("failed");
          return;
        }
        setEmbedSessionPhase("authorizing");
        void openUrl(loginUrl).catch(() => {
          setBusinessAuth({
            phase: "failed",
            reason: "Failed to open the system-browser SSO flow.",
          });
          setEmbedSessionPhase("failed");
          toast.error("Failed to open business sign-in");
        });
        return;
      }

      const loginUrl = new URL("/auth/login", config.origin);
      loginUrl.searchParams.set("popup", "1");
      const popup = window.open(
        loginUrl.href,
        "business-sso",
        "popup,width=620,height=720",
      );
      if (!popup) {
        setBusinessAuth({
          phase: "failed",
          reason: "The Business sign-in popup was blocked.",
        });
        return;
      }
      setEmbedSessionPhase("idle");
      if (popupPollRef.current) window.clearInterval(popupPollRef.current);
      popupPollRef.current = window.setInterval(() => {
        if (!popup.closed) return;
        if (popupPollRef.current) window.clearInterval(popupPollRef.current);
        popupPollRef.current = null;
        setBusinessAuth({ phase: "checking" });
        const frame = iframeRef.current;
        if (frame) frame.src = targetUrl;
      }, 250);
      if (automatic) debugBusinessDock("Business recovery started");
    },
    [
      config,
      ssoMode,
      workbenchAuth.gatewayState.status,
      workbenchAuth.gatewayUrl,
      workbenchAuth.getAccessToken,
      workbenchAuth.phase,
    ],
  );

  const startBusinessSignIn = React.useCallback(() => {
    recoveryAttemptsRef.current = 0;
    beginBusinessSignIn(false);
  }, [beginBusinessSignIn]);

  React.useEffect(() => {
    if (!config || ssoMode !== "desktop-embed-session") return;
    let disposed = false;
    let cleanup: () => void = () => undefined;
    void subscribeToBusinessEmbedCallbacks((code) => {
      if (disposed) return;
      const bootstrapUrl = buildBusinessEmbedBootstrapUrl(config, code);
      if (!bootstrapUrl || !iframeRef.current) {
        setBusinessAuth({
          phase: "failed",
          reason: "The one-time Embed Session callback was rejected.",
        });
        setEmbedSessionPhase("failed");
        return;
      }
      setBusinessAuth({ phase: "checking" });
      setEmbedSessionPhase("redeeming");
      iframeRef.current.src = bootstrapUrl;
      if (embedTimeoutRef.current) window.clearTimeout(embedTimeoutRef.current);
      embedTimeoutRef.current = window.setTimeout(() => {
        setBusinessAuth((current) =>
          current.phase === "authenticated"
            ? current
            : {
                phase: "failed",
                reason: "The one-time Embed Session could not be redeemed.",
              },
        );
        setEmbedSessionPhase((current) =>
          current === "ready" ? current : "failed",
        );
      }, 8_000);
    }).then((unsubscribe) => {
      if (disposed) unsubscribe();
      else cleanup = unsubscribe;
    });
    return () => {
      disposed = true;
      cleanup();
    };
  }, [config, ssoMode]);

  React.useEffect(() => {
    if (
      businessAuth.phase !== "expired" ||
      workbenchAuth.phase !== "authenticated" ||
      !canAttemptBusinessRecovery(recoveryAttemptsRef.current)
    )
      return;
    recoveryAttemptsRef.current += 1;
    beginBusinessSignIn(true);
  }, [beginBusinessSignIn, businessAuth.phase, workbenchAuth.phase]);

  React.useEffect(() => {
    if (!config) return;
    const onMessage = (event: MessageEvent) => {
      const message = readBusinessBridgeEvent(
        event,
        iframeRef.current?.contentWindow ?? null,
        config,
        sessionNonceRef.current,
      );
      if (!message) return;
      if (message.type === "AUTH_STATUS") {
        if (message.payload.authenticated) {
          recoveryAttemptsRef.current = 0;
          if (embedTimeoutRef.current)
            window.clearTimeout(embedTimeoutRef.current);
          embedTimeoutRef.current = null;
          if (ssoMode === "desktop-embed-session")
            setEmbedSessionPhase("ready");
        }
        setBusinessAuth(
          message.payload.authenticated
            ? {
                phase: "authenticated",
                identity: {
                  subject: message.payload.user.subject,
                  displayName: message.payload.user.displayName,
                },
              }
            : { phase: "unconnected" },
        );
        return;
      }
      if (message.type === "AUTH_REQUIRED") {
        setBusinessAuth((current) =>
          current.phase === "signing-in"
            ? current
            : {
                phase: "unconnected",
                ...(message.payload.reason
                  ? { reason: message.payload.reason }
                  : {}),
              },
        );
        return;
      }
      if (message.type === "SESSION_EXPIRED") {
        setBusinessAuth({
          phase: "expired",
          ...(message.payload.reason ? { reason: message.payload.reason } : {}),
        });
        return;
      }
      if (message.type === "BUSINESS_READY") {
        setBridgeVersion(message.version);
        debugBusinessDock("Business Bridge connected");
        if (message.version === 2)
          postV2("SET_THEME", { theme: isDark ? "dark" : "light" });
        else postV1("SET_THEME", { theme: isDark ? "dark" : "light" });
        const pending = pendingNavigationRef.current;
        if (pending) {
          pendingNavigationRef.current = null;
          if (message.version === 2) postV2("NAVIGATE", { resource: pending });
          else {
            const url = buildBusinessUrl(pending, config);
            if (url) postV1("NAVIGATE", { url });
          }
        }
        return;
      }
      if (message.type === "TITLE_CHANGED") {
        dispatch({ type: "title", title: message.payload.title });
        return;
      }
      if (message.type === "DIRTY_STATE_CHANGED") {
        dispatch({ type: "dirty", dirty: message.payload.dirty });
        return;
      }
      if (message.type === "RESOURCE_CHANGED") {
        const resource = message.payload.resource;
        const url = buildBusinessUrl(resource, config);
        if (!url) return;
        dispatch({ type: "navigate", url, resource, openingResource: false });
        dispatch({ type: "resource", resource });
        setNavigation((current) =>
          updateCurrentBusinessNavigation(current, resource),
        );
        return;
      }
      if (message.type === "ROUTE_CHANGED") {
        const resource = parseBusinessUrl(message.payload.url, config);
        if (!resource) return;
        dispatch({
          type: "navigate",
          url: message.payload.url,
          resource,
          openingResource: false,
        });
        dispatch({ type: "resource", resource });
        setNavigation((current) =>
          updateCurrentBusinessNavigation(current, resource),
        );
        return;
      }
      if (
        message.type === "ACTION_COMPLETED" ||
        message.type === "ACTION_FAILED"
      ) {
        const failed = message.type === "ACTION_FAILED";
        dispatch({
          type: "action",
          status: failed ? "failed" : "completed",
          action: message.payload.action,
          message: message.payload.message,
          ...(message.payload.traceId
            ? { traceId: message.payload.traceId }
            : {}),
        });
        if (failed) toast.error(message.payload.message);
        else toast.success(message.payload.message);
        return;
      }
      if (message.type !== "DATA_CHANGED") return;
      dispatch({ type: "data-changed", changed: true });
      window.dispatchEvent(
        new CustomEvent(BUSINESS_DATA_CHANGED_EVENT, {
          detail: message.payload.resource
            ? {
                type: message.payload.resource.type,
                id: message.payload.resource.id,
              }
            : {},
        }),
      );
    };
    window.addEventListener("message", onMessage);
    return () => window.removeEventListener("message", onMessage);
  }, [config, isDark, postV1, postV2, ssoMode]);

  React.useEffect(() => {
    if (bridgeVersion === 2)
      postV2("SET_THEME", { theme: isDark ? "dark" : "light" });
    else if (bridgeVersion === 1)
      postV1("SET_THEME", { theme: isDark ? "dark" : "light" });
  }, [bridgeVersion, isDark, postV1, postV2]);

  React.useEffect(() => {
    const onKeyDown = (event: KeyboardEvent) => {
      if (
        event.key.toLowerCase() === "b" &&
        event.shiftKey &&
        (event.metaKey || event.ctrlKey)
      ) {
        event.preventDefault();
        if (stateRef.current.open)
          requestLeave(() => dispatch({ type: "close" }));
        else dispatch({ type: "open" });
      }
    };
    const onBeforeUnload = (event: BeforeUnloadEvent) => {
      if (!stateRef.current.dirty) return;
      event.preventDefault();
      event.returnValue = "";
    };
    window.addEventListener("keydown", onKeyDown);
    window.addEventListener("beforeunload", onBeforeUnload);
    return () => {
      window.removeEventListener("keydown", onKeyDown);
      window.removeEventListener("beforeunload", onBeforeUnload);
    };
  }, [requestLeave]);

  const moveHistory = React.useCallback(
    (direction: -1 | 1) => {
      const current = navigationRef.current;
      if (!canMoveBusinessNavigation(current, direction)) return;
      const next = moveBusinessNavigation(current, direction);
      const resource = currentBusinessNavigationEntry(next);
      if (!resource) return;
      requestLeave(() => {
        setNavigation(next);
        commitNavigation(resource, "history");
      });
    },
    [commitNavigation, requestLeave],
  );

  const onBrowserLoad = React.useCallback(() => {
    dispatch({ type: "loading", loading: false });
    postV2("HOST_INIT", { hostVersion: 2 });
    postV1("HOST_INIT", { version: 1 });
    postV3("HOST_INIT", { hostVersion: 3 });
    setBusinessAuth({ phase: "checking" });
    postV3("CHECK_AUTH", { interactive: false });
  }, [postV1, postV2, postV3]);

  const value = React.useMemo<BusinessDockContextValue>(
    () => ({
      bridgeReady: bridgeVersion !== null,
      businessAuth,
      embedSessionPhase,
      ssoMode,
      workbenchAuthPhase: workbenchAuth.phase,
      workbenchGatewayStatus: workbenchAuth.gatewayState.status,
      workbenchGroupClaimStatus: workbenchAuth.groupClaimStatus,
      canGoBack: canMoveBusinessNavigation(navigation, -1),
      canGoForward: canMoveBusinessNavigation(navigation, 1),
      canResetWidth: width.canReset,
      close: () => requestLeave(() => dispatch({ type: "close" })),
      checkBusinessAuth: () => {
        setBusinessAuth({ phase: "checking" });
        postV3("CHECK_AUTH", { interactive: false });
      },
      config,
      configError: configResult.error,
      getCurrentBusinessResource: () => stateRef.current.currentResource,
      goBack: () => moveHistory(-1),
      goForward: () => moveHistory(1),
      goHome: () => {
        if (initialResource) openBusinessResource(initialResource);
      },
      iframeRef,
      isOverlay: width.isOverlay,
      navigateBusinessResource: openBusinessResource,
      onBrowserLoad,
      onResetWidth: width.onResetWidth,
      onResizeStart: width.onResizeStart,
      openBusinessResource,
      openBusinessResourceInBrowser: (resource) => {
        if (!config) return;
        const url = buildBusinessUrl(resource, config);
        if (!url) return;
        requestLeave(() => {
          void openUrl(url).catch(() =>
            toast.error("Failed to open business page"),
          );
        });
      },
      openBusinessResourceLink: (input) => {
        if (!config) return false;
        const resource = resolveBusinessResource(input, config);
        if (!resource) return false;
        openBusinessResource(resource);
        return true;
      },
      openCurrentInBrowser: () => {
        if (!config) return;
        const resource = stateRef.current.currentResource;
        const url = resource
          ? buildBusinessUrl(resource, config)
          : stateRef.current.currentUrl;
        if (!url || !resolveBusinessResource(url, config)) return;
        requestLeave(() => {
          void openUrl(url).catch(() =>
            toast.error("Failed to open business page"),
          );
        });
      },
      logoutBusiness: () => {
        setBusinessAuth({ phase: "checking" });
        postV3("LOGOUT");
      },
      refresh: () => {
        dispatch({ type: "loading", loading: true });
        if (bridgeVersion === 2 && postV2("REFRESH")) return;
        if (bridgeVersion === 1 && postV1("REFRESH")) return;
        const frame = iframeRef.current;
        if (frame && config)
          frame.src = stateRef.current.currentUrl || config.homeUrl;
      },
      renderedWidthPx: width.renderedWidthPx,
      requestCurrentBusinessResource: () => {
        if (bridgeVersion === 2) postV2("REQUEST_CURRENT_RESOURCE");
      },
      resolveBusinessResourceLink: (input) =>
        config ? resolveBusinessResource(input, config) : null,
      setBusinessContext: (context) => {
        if (bridgeVersion !== 2) return;
        const legalEntityId = context.legalEntityId?.trim();
        const period = context.period?.trim();
        if (
          legalEntityId &&
          !/^[A-Za-z0-9][A-Za-z0-9._:@-]{0,127}$/.test(legalEntityId)
        )
          return;
        if (period && !/^\d{4}-(?:0[1-9]|1[0-2])$/.test(period)) return;
        postV2("SET_CONTEXT", {
          ...(legalEntityId ? { legalEntityId } : {}),
          ...(period ? { period } : {}),
        });
      },
      startBusinessSignIn,
      bindCurrentDevice: () => void workbenchAuth.bindCurrentDevice(),
      state,
      toggle: () => {
        if (stateRef.current.open)
          requestLeave(() => dispatch({ type: "close" }));
        else dispatch({ type: "open" });
      },
      toggleFollowConversation: () => dispatch({ type: "toggle-follow" }),
      toggleFullscreen: () => {
        if (stateRef.current.fullscreen)
          requestLeave(() => dispatch({ type: "exit-fullscreen" }));
        else dispatch({ type: "toggle-fullscreen" });
      },
      togglePinned: () => dispatch({ type: "toggle-pinned" }),
    }),
    [
      bridgeVersion,
      businessAuth,
      config,
      configResult.error,
      embedSessionPhase,
      initialResource,
      moveHistory,
      navigation,
      onBrowserLoad,
      openBusinessResource,
      postV1,
      postV2,
      postV3,
      requestLeave,
      ssoMode,
      startBusinessSignIn,
      state,
      width,
      workbenchAuth.phase,
      workbenchAuth.groupClaimStatus,
      workbenchAuth.gatewayState.status,
      workbenchAuth.bindCurrentDevice,
    ],
  );

  const confirmLeave = () => {
    const action = pendingLeaveRef.current;
    pendingLeaveRef.current = null;
    setLeaveConfirmationOpen(false);
    dispatch({ type: "dirty", dirty: false });
    action?.();
  };

  return (
    <BusinessDockContext.Provider value={value}>
      {children}
      <AlertDialog
        open={leaveConfirmationOpen}
        onOpenChange={(open) => {
          setLeaveConfirmationOpen(open);
          if (!open) pendingLeaveRef.current = null;
        }}
      >
        <AlertDialogContent data-testid="business-dock-dirty-dialog">
          <AlertDialogHeader>
            <AlertDialogTitle>当前业务页面存在未保存更改。</AlertDialogTitle>
            <AlertDialogDescription>
              离开后这些修改可能丢失。
            </AlertDialogDescription>
          </AlertDialogHeader>
          <AlertDialogFooter>
            <AlertDialogCancel>取消</AlertDialogCancel>
            <AlertDialogAction onClick={confirmLeave}>
              仍然离开
            </AlertDialogAction>
          </AlertDialogFooter>
        </AlertDialogContent>
      </AlertDialog>
    </BusinessDockContext.Provider>
  );
}

export function useBusinessDock() {
  const context = React.useContext(BusinessDockContext);
  if (!context)
    throw new Error("useBusinessDock must be used within BusinessDockProvider");
  return context;
}

export function useOptionalBusinessDock() {
  return React.useContext(BusinessDockContext);
}
