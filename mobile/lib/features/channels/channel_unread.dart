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
  // Lists rebuild far more often than unread evidence changes, and a backlog
  // can hold a thousand events per channel: reuse each channel's outcome
  // until its events or the read state change.
  final revisionOf = observedEventsByChannel is ObservedUnreadSnapshot
      ? observedEventsByChannel.revisionOf
      : null;
  final memo = revisionOf == null
      ? null
      : (_memos[channelsNotifier] ??= _HighlightMemo()).entriesFor(
          readState,
          separateAttention: separateAttention,
        );

  for (final channel in channels) {
    final latestObserved = latestObservedByChannel[channel.id];
    final revision = revisionOf?.call(channel.id);
    final cached = memo?[channel.id];
    final int flags;
    if (cached != null &&
        cached.revision == revision &&
        cached.latestObserved == latestObserved &&
        cached.isDm == channel.isDm) {
      flags = cached.flags;
    } else {
      flags = _channelHighlights(
        channel: channel,
        readState: readState,
        forced: forcedChannelIds.contains(channel.id),
        latestObserved: latestObserved,
        observedEvents: observedEventsByChannel[channel.id],
        separateAttention: separateAttention,
      );
      if (memo != null && revision != null) {
        memo[channel.id] = (
          revision: revision,
          latestObserved: latestObserved,
          isDm: channel.isDm,
          flags: flags,
        );
      }
    }
    if (flags & _unread != 0) unread.add(channel.id);
    if (flags & _rowUnread != 0) rowUnread.add(channel.id);
    if (flags & _attention != 0) attention.add(channel.id);
  }

  return highlights;
}

const _unread = 1;
const _rowUnread = 2;
const _attention = 4;

/// One channel's highlight flags. Stops reading events once no later event
/// could change the outcome.
int _channelHighlights({
  required Channel channel,
  required ReadStateState readState,
  required bool forced,
  required int? latestObserved,
  required Map<String, ObservedUnreadEvent>? observedEvents,
  required bool separateAttention,
}) {
  var flags = 0;
  if (forced) {
    flags |= _unread;
    if (separateAttention) flags |= _rowUnread;
  }
  // Every flag an event could still set. Without separate attention only
  // `unread` exists; a DM's first unread event sets all three at once.
  final decided = separateAttention
      ? _unread | _rowUnread | _attention
      : _unread;
  if (flags == decided) return flags;

  if (latestObserved == null) return flags;
  final channelReadAt = readState.effectiveTimestamp(channel.id);
  if (channelReadAt != null && latestObserved <= channelReadAt) return flags;
  if (observedEvents == null) return flags;

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
      flags |= _unread;
      if (separateAttention &&
          (channel.isDm || seenAt == null || event.createdAt > seenAt)) {
        flags |= _rowUnread;
      }
    }
    if (separateAttention && (channel.isDm || event.attention)) {
      flags |= _attention;
    }
    if (flags == decided) break;
  }
  return flags;
}

typedef _MemoEntry = ({
  int revision,
  int? latestObserved,
  bool isDm,
  int flags,
});

/// Per-notifier memo, so it ends with the notifier's community and identity.
final _memos = Expando<_HighlightMemo>();

/// Channel outcomes for one read state, per `separateAttention` mode. Read
/// state is immutable and replaced on every change, so identity is the key.
class _HighlightMemo {
  ReadStateState? _readState;
  final _combined = <String, _MemoEntry>{};
  final _separate = <String, _MemoEntry>{};

  Map<String, _MemoEntry> entriesFor(
    ReadStateState readState, {
    required bool separateAttention,
  }) {
    if (!identical(_readState, readState)) {
      _readState = readState;
      _combined.clear();
      _separate.clear();
    }
    return separateAttention ? _separate : _combined;
  }
}
