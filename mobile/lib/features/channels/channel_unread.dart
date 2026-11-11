import '../../shared/read_state/read_state_format.dart';
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
/// `docs/unread.md`). Ordinary unread backlog bolds a channel ([unread]).
/// Unread thread replies, mentions and broadcasts only add an attention dot
/// ([attention]), so a channel the reader caught up on stays regular. An
/// unread DM bolds and dots. Marking a channel unread bolds it without
/// fabricating attention, but keeps any real attention's dot. What the row
/// has seen ([rowSawObservedUnreadEvent]) neither bolds nor dots it, unless
/// the reader marked the channel or one of its messages unread.
({Set<String> unread, Set<String> attention}) computeChannelListHighlights({
  required Iterable<Channel> channels,
  required ReadStateState readState,
  required ChannelsNotifier channelsNotifier,
}) => _computeHighlights(
  channels: channels,
  readState: readState,
  channelsNotifier: channelsNotifier,
  separateAttention: true,
  rowCut: true,
);

({Set<String> unread, Set<String> attention}) _computeHighlights({
  required Iterable<Channel> channels,
  required ReadStateState readState,
  required ChannelsNotifier channelsNotifier,
  required bool separateAttention,
  bool rowCut = false,
}) {
  final unread = <String>{};
  final attention = <String>{};
  if (!readState.isReady) return (unread: unread, attention: attention);

  final latestObservedByChannel = channelsNotifier.latestObservedByChannel;
  final observedEventsByChannel =
      channelsNotifier.observedUnreadEventsByChannel;

  for (final channel in channels) {
    if (readState.locallyForcedChannelIds.contains(channel.id)) {
      unread.add(channel.id);
    }

    final latestObserved = latestObservedByChannel[channel.id];
    if (latestObserved == null) continue;

    final channelReadAt = readState.effectiveTimestamp(channel.id);
    if (channelReadAt != null && latestObserved <= channelReadAt) continue;

    final observedEvents = observedEventsByChannel[channel.id];
    if (observedEvents == null) continue;
    final cut =
        rowCut && !readState.locallyForcedChannelIds.contains(channel.id);
    for (final event in observedEvents.values) {
      final readAt = observedUnreadEventReadAt(
        event,
        channel.id,
        readState.effectiveTimestamp,
      );
      if (readAt != null && event.createdAt <= readAt) continue;
      if (cut &&
          rowSawObservedUnreadEvent(
            event,
            channel.id,
            (key) => readState.contexts[key],
          )) {
        continue;
      }
      if (!separateAttention || channel.isDm || event.channelCatchUp) {
        unread.add(channel.id);
      }
      if (separateAttention && (channel.isDm || !event.channelCatchUp)) {
        attention.add(channel.id);
      }
    }
  }

  return (unread: unread, attention: attention);
}

/// The observed events [channelId]'s row and the app badge still show: the
/// unread ones its row has not seen, or all of them when the reader marked
/// the channel or one of its messages unread.
Map<String, ObservedUnreadEvent> rowObservedUnreadEvents(
  Map<String, ObservedUnreadEvent> events,
  String channelId,
  ReadStateState readState,
) {
  if (readState.locallyForcedChannelIds.contains(channelId)) return events;
  return {
    for (final MapEntry(key: id, value: event) in events.entries)
      if (!rowSawObservedUnreadEvent(
        event,
        channelId,
        (key) => readState.contexts[key],
      ))
        id: event,
  };
}

/// The row hints that quiet [channelId]'s row through what it shows now,
/// without reading anything: a receipt for each attention message and the
/// newest shown time for ordinary ones. Empty when the row shows nothing
/// the saved hints do not already hide.
Map<String, int> channelRowSeenMarks({
  required String channelId,
  required Map<String, ObservedUnreadEvent> events,
  required ReadStateState readState,
}) {
  final marks = <String, int>{};
  var newest = -1;
  var ordinary = false;
  for (final event in events.values) {
    final readAt = observedUnreadEventReadAt(
      event,
      channelId,
      readState.effectiveTimestamp,
    );
    if (readAt != null && event.createdAt <= readAt) continue;
    if (rowSawObservedUnreadEvent(
      event,
      channelId,
      (key) => readState.contexts[key],
    )) {
      continue;
    }
    if (event.createdAt > newest) newest = event.createdAt;
    if (event.channelCatchUp) {
      ordinary = true;
    } else {
      marks[seenMessageContextKey(event.id)] = event.createdAt;
    }
  }
  if (ordinary) marks[seenContextKey(channelId)] = newest;
  return marks;
}
