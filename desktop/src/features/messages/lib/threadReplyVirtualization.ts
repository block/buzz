/**
 * Small threads stay fully mounted so their existing branch interactions keep
 * the simplest possible DOM. Large threads are windowed before WebKit has to
 * maintain hundreds of rich message rows in one scrolling tree.
 */
export const THREAD_REPLY_VIRTUALIZATION_THRESHOLD = 50;

export const THREAD_REPLY_ESTIMATED_HEIGHT_PX = 88;

export function shouldVirtualizeThreadReplies(replyCount: number): boolean {
  return replyCount > THREAD_REPLY_VIRTUALIZATION_THRESHOLD;
}
