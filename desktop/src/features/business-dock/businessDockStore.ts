import type { BusinessDockConfig } from "@/features/business-dock/businessDockConfig";
import type { BusinessDockPreferences } from "@/features/business-dock/businessDockPreferences";
import type { BusinessResource } from "@/features/business-dock/businessResourceResolver";

export type BusinessDockState = {
  currentUrl: string;
  currentResource: BusinessResource | null;
  dataChanged: boolean;
  dirty: boolean;
  followConversation: boolean;
  fullscreen: boolean;
  loading: boolean;
  open: boolean;
  openingResource: boolean;
  pinned: boolean;
  lastAction?: {
    status: "completed" | "failed";
    action: string;
    message: string;
    traceId?: string;
  };
  title?: string;
};

export type BusinessDockAction =
  | { type: "toggle" }
  | { type: "open" }
  | { type: "close" }
  | { type: "toggle-fullscreen" }
  | { type: "exit-fullscreen" }
  | { type: "toggle-pinned" }
  | { type: "toggle-follow" }
  | { type: "loading"; loading: boolean }
  | {
      type: "navigate";
      url: string;
      resource?: BusinessResource | null;
      openingResource?: boolean;
    }
  | { type: "resource"; resource: BusinessResource }
  | { type: "dirty"; dirty: boolean }
  | { type: "data-changed"; changed: boolean }
  | {
      type: "action";
      status: "completed" | "failed";
      action: string;
      message: string;
      traceId?: string;
    }
  | { type: "title"; title?: string };

export function createInitialBusinessDockState(
  config: BusinessDockConfig | null,
  preferences: BusinessDockPreferences = {
    followConversation: true,
    pinned: false,
  },
): BusinessDockState {
  return {
    currentUrl: config?.homeUrl ?? "",
    currentResource: null,
    dataChanged: false,
    dirty: false,
    followConversation: preferences.followConversation,
    fullscreen: false,
    loading: false,
    open: false,
    openingResource: false,
    pinned: preferences.pinned,
    title: undefined,
  };
}

export function businessDockReducer(
  state: BusinessDockState,
  action: BusinessDockAction,
): BusinessDockState {
  switch (action.type) {
    case "toggle":
      return state.open
        ? { ...state, fullscreen: false, open: false }
        : { ...state, open: true };
    case "open":
      return { ...state, open: true };
    case "close":
      return { ...state, fullscreen: false, open: false };
    case "toggle-fullscreen":
      return state.open
        ? { ...state, fullscreen: !state.fullscreen }
        : { ...state, fullscreen: true, open: true };
    case "exit-fullscreen":
      return { ...state, fullscreen: false };
    case "toggle-pinned":
      return { ...state, pinned: !state.pinned };
    case "toggle-follow":
      return { ...state, followConversation: !state.followConversation };
    case "loading":
      return { ...state, loading: action.loading };
    case "navigate":
      return {
        ...state,
        currentUrl: action.url,
        currentResource:
          action.resource === undefined
            ? state.currentResource
            : action.resource,
        loading: true,
        openingResource: action.openingResource ?? state.openingResource,
      };
    case "resource":
      return {
        ...state,
        currentResource: action.resource,
        dataChanged: false,
        loading: false,
        openingResource: false,
      };
    case "dirty":
      return { ...state, dirty: action.dirty };
    case "data-changed":
      return { ...state, dataChanged: action.changed };
    case "action":
      return {
        ...state,
        lastAction: {
          status: action.status,
          action: action.action,
          message: action.message,
          ...(action.traceId ? { traceId: action.traceId } : {}),
        },
      };
    case "title":
      return { ...state, title: action.title };
  }
}
