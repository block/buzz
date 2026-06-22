export function shouldShowSidebarUpdateCard(status: { state: string }): boolean {
  return status.state === "ready";
}
