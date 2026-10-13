// Web replacement for the native "Open in a new window" pop-out: open the channel/thread in a new browser tab
// using the desktop's pop-out layout (no sidebar). The tab URL is `/?popout=<label>#/channels/<id>[?thread=..]`;
// the pop-out payload lives in localStorage under that label, exactly like the desktop's OS pop-out windows.
// Opened with noopener, so the new tab gets a fresh sessionStorage (the bridge key cache is never copied);
// the signer session lives in localStorage and is shared by the new tab automatically.
export async function openConversationInNewTab(input: {
  channelId: string;
  threadId?: string;
  title?: string;
}): Promise<void> {
  const { router } = await import("@/app/router");
  const { popoutLabel, popoutPayloadFromInput, writePopoutPayload, WEB_POPOUT_QUERY_PARAM } =
    await import("@/features/popout/lib/popoutWindow");
  const loc = router.buildLocation({
    to: "/channels/$channelId",
    params: { channelId: input.channelId },
    search: input.threadId ? { thread: input.threadId } : {},
  } as never);
  const label = popoutLabel(
    "thread",
    `${input.channelId}:${input.threadId ?? ""}:${Date.now()}`,
  );
  writePopoutPayload(
    label,
    popoutPayloadFromInput({
      kind: "thread",
      title: input.title ?? "HulaBuzz",
      channelId: input.channelId,
      threadId: input.threadId,
    }),
  );
  const href = router.history.createHref(loc.href);
  const url = new URL(href, window.location.href);
  // Keep page-level params (e.g. `?relay=`) in the real query, before the `#` route, never appended after it.
  for (const [k, v] of new URLSearchParams(window.location.search)) if (k !== WEB_POPOUT_QUERY_PARAM && !url.searchParams.has(k)) url.searchParams.set(k, v);
  url.searchParams.set(WEB_POPOUT_QUERY_PARAM, label);
  const w = window.open(url.toString(), "_blank", "noopener,noreferrer");
  if (!w && w !== null) throw new Error("Pop-up blocked");
}
