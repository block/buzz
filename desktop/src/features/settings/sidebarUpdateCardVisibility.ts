export function shouldShowSidebarUpdateCard(status: { state: string }) {
  return status.state === "ready";
}
