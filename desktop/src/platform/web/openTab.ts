// Web replacement for the native "Open in a new window" pop-out: open the same
// channel/thread route in a new browser tab, using the app's own router to build
// the hash URL. Opened with noopener, so the new tab gets a fresh sessionStorage
// (secrets such as the bridge key cache are never copied); the signer session
// lives in localStorage and is shared by the new tab automatically.
export async function openConversationInNewTab(input: {
  channelId: string;
  threadId?: string;
}): Promise<void> {
  const { router } = await import("@/app/router");
  const loc = router.buildLocation({
    to: "/channels/$channelId",
    params: { channelId: input.channelId },
    search: input.threadId ? { thread: input.threadId } : {},
  } as never);
  const href = router.history.createHref(loc.href);
  const url = new URL(href, window.location.href).toString();
  const w = window.open(url, "_blank", "noopener,noreferrer");
  if (!w && w !== null) throw new Error("Pop-up blocked");
}
