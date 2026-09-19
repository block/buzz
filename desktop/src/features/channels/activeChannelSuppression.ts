/**
 * Decides whether a notification for `channelId` is suppressed because the
 * user is actively viewing that channel.
 *
 * "Viewing" requires the app window to be focused: a channel that is merely
 * selected in a minimized or backgrounded window is not being read, so
 * notifications for it must still fire. `notifyForActiveChannel` opts out of
 * suppression entirely.
 */
export function isSuppressedAsActiveChannel(
  channelId: string,
  activeChannelId: string | null,
  appFocused: boolean,
  notifyForActiveChannel: boolean,
): boolean {
  if (notifyForActiveChannel) {
    return false;
  }
  return channelId === activeChannelId && appFocused;
}
