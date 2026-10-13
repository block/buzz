import { openExternalUrl } from "@/shared/lib/externalLinks";
import { setWebDocumentTitle } from "./documentTitle";
// window / webview / app / path / plugin shims (web: no-ops or browser equivalents)
const noop = async () => undefined;
const win = new Proxy(
  {},
  {
    get: (_t, p) =>
      p === "label"
        ? "main"
        : p === "isFocused"
          ? async () => document.hasFocus()
          : p === "isVisible"
            ? async () => true
            : p === "setTitle"
              ? async (title?: string) => {
                  setWebDocumentTitle(title);
                }
              : p === "onFocusChanged" ||
                  p === "onResized" ||
                  p === "onMoved" ||
                  p === "onCloseRequested" ||
                  p === "listen" ||
                  p === "onDragDropEvent"
                ? async () => () => {}
                : noop,
  },
);
export const getCurrentWindow = () => win as any;
export const getCurrentWebview = () => win as any;
export class WebviewWindow {
  constructor(
    public label: string,
    _o?: unknown,
  ) {}
  static getByLabel = async () => null;
  once = noop;
  listen = async () => () => {};
}
export const UserAttentionType = { Critical: 1, Informational: 2 } as const;
export const getVersion = async () =>
  (import.meta.env.VITE_APP_VERSION as string) || "web";
export const homeDir = async () => "/";
export const openUrl = async (url: string) => {
  openExternalUrl(url);
};
export const relaunch = async () => location.reload();
export const isPermissionGranted = async () =>
  typeof Notification !== "undefined" && Notification.permission === "granted";
export const requestPermission = async () =>
  typeof Notification === "undefined"
    ? "denied"
    : Notification.requestPermission();
export const onAction = async () => ({ unregister: async () => {} });
export const check = async () => null;
export class Update {}
