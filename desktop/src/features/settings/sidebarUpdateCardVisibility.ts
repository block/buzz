export function shouldShowSidebarUpdateCard(status: { state: string }) {
  return (
    status.state === "ready" ||
    status.state === "installing" ||
    status.state === "manual-required"
  );
}
