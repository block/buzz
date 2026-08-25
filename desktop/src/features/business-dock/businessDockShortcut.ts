export function isBusinessDockShortcut(
  event: Pick<
    KeyboardEvent,
    "altKey" | "ctrlKey" | "key" | "metaKey" | "shiftKey"
  >,
): boolean {
  return (
    event.key.toLowerCase() === "b" &&
    event.shiftKey &&
    (event.metaKey || event.ctrlKey) &&
    !event.altKey
  );
}
