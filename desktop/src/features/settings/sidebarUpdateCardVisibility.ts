const SHOW_UPDATE_CARD_PREVIEW =
  import.meta.env.DEV &&
  import.meta.env.VITE_SIDEBAR_UPDATE_CARD_PREVIEW === "1";

export function shouldShowSidebarUpdateCard(status: { state: string }) {
  return status.state === "ready" || SHOW_UPDATE_CARD_PREVIEW;
}
