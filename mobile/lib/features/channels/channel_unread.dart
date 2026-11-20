import '../../shared/read_state/read_state_provider.dart';
import 'channel.dart';
import 'channels_provider.dart';
import 'unread_badge/observed_unread_event.dart';

/// Shared channel highlights, including reply and individual-message read marks.
/// Every unread event highlights the channel, as Solo chats need.
Set<String> computeUnreadChannelIds({
  required Iterable<Channel> channels,
  required ReadStateState readState,
  required ChannelsNotifier channelsNotifier,
}) => _computeHighlights(
  channels: channels,
  readState: readState,
  channelsNotifier: channelsNotifier,
  separateAttention: false,
).unread;

/// Channel-list highlights, following the desktop sidebar (`buzz-app`'s
/// `docs/unread.md`). Ordinary unread backlog bolds a channel (`rowUnread`)
/// unless a synced `seen:<channel>` mark covers it. `unread` keeps the
/// underlying evidence for read actions.
/// Unread thread replies and mentions only add an attention dot
/// ([attention]), so a channel the reader caught up on stays regular. A reply
/// also sent to the channel dots only in one of the reader's threads;
/// elsewhere it bolds, like a top-level post. An
/// unread DM bolds and dots. Marking a channel unread bolds it without
/// fabricating attention, but keeps any real attention's dot.
({Set<String> unread, Set<String> rowUnread, Set<String> attention})
computeChannelListHighlights({
  required Iterable<Channel> channels,
  required ReadStateState readState,
  required ChannelsNotifier channelsNotifier,
}) => _computeHighlights(
  channels: channels,
  readState: readState,
  channelsNotifier: channelsNotifier,
  separateAttention: true,
);

({Set<String> unread, Set<String> rowUnread, Set<String> attention})
_computeHighlights({
  required Iterable<Channel> channels,
  required ReadStateState readState,
  required ChannelsNotifier channelsNotifier,
  required bool separateAttention,
}) {
  final unread = <String>{};
  final rowUnread = <String>{};
  final attention = <String>{};
  final highlights = (
    unread: unread,
    rowUnread: rowUnread,
    attention: attention,
  );
  if (!readState.isReady) return highlights;

  final forcedChannelIds = readState.locallyForcedChannelIds;
  final latestObservedByChannel = channelsNotifier.latestObservedByChannel;
  final observedEventsByChannel =
      channelsNotifier.observedUnreadEventsByChannel;

  for (final channel in channels) {
    if (forcedChannelIds.contains(channel.id)) {
      unread.add(channel.id);
      if (separateAttention) rowUnread.add(channel.id);
    }

    final latestObserved = latestObservedByChannel[channel.id];
    if (latestObserved == null) continue;

    final channelReadAt = readState.effectiveTimestamp(channel.id);
    if (channelReadAt != null && latestObserved <= channelReadAt) continue;

    final observedEvents = observedEventsByChannel[channel.id];
    if (observedEvents == null) continue;
    final seenAt = separateAttention
        ? readState.contexts['seen:${channel.id}']
        : null;
    for (final event in observedEvents.values) {
      final readAt = observedUnreadEventReadAt(
        event,
        channel.id,
        readState.effectiveTimestamp,
      );
      if (readAt != null && event.createdAt <= readAt) continue;
      if (!separateAttention || channel.isDm || !event.attention) {
        unread.add(channel.id);
        if (separateAttention &&
            (channel.isDm || seenAt == null || event.createdAt > seenAt)) {
          rowUnread.add(channel.id);
        }
      }
      if (separateAttention && (channel.isDm || event.attention)) {
        attention.add(channel.id);
      }
    }
  }

  return highlights;
}
