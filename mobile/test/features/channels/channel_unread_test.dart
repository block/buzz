import 'package:buzz/features/channels/channel.dart';
import 'package:buzz/features/channels/channel_unread.dart';
import 'package:buzz/features/channels/channels_provider.dart';
import 'package:buzz/features/channels/unread_badge/observed_unread_event.dart';
import 'package:buzz/shared/read_state/read_state_provider.dart';
import 'package:flutter_test/flutter_test.dart';

Channel _channel(String id, {String type = 'stream'}) => Channel(
  id: id,
  name: id,
  channelType: type,
  visibility: 'open',
  description: '',
  createdBy: 'owner',
  createdAt: DateTime(2026),
  memberCount: 2,
  isMember: true,
);

ObservedUnreadEvent _event(
  String id, {
  String type = 'stream',
  bool reply = false,
  bool mention = false,
}) => makeObservedUnreadEvent(
  id: id,
  createdAt: 20,
  rootId: reply ? 'root' : null,
  highPriority: mention,
  channelType: type,
  isThreadedReply: reply,
);

class _Channels extends ChannelsNotifier {
  final Map<String, List<ObservedUnreadEvent>> events;
  _Channels(this.events);

  @override
  Map<String, int> get latestObservedByChannel => {
    for (final id in events.keys) id: 20,
  };

  @override
  Map<String, Map<String, ObservedUnreadEvent>>
  get observedUnreadEventsByChannel => {
    for (final MapEntry(:key, :value) in events.entries)
      key: {for (final event in value) event.id: event},
  };
}

void main() {
  final channels = [
    _channel('backlog'),
    _channel('reply'),
    _channel('mention'),
    _channel('both'),
    _channel('dm', type: 'dm'),
    _channel('forced'),
    _channel('forced-backlog'),
  ];
  final notifier = _Channels({
    'backlog': [_event('m1')],
    'reply': [_event('r1', reply: true)],
    'mention': [_event('p1', mention: true)],
    'both': [_event('m2'), _event('r2', reply: true)],
    'dm': [_event('d1', type: 'dm', reply: true)],
    'forced': [_event('p2', mention: true)],
    'forced-backlog': [_event('m3')],
  });
  const readState = ReadStateState(
    isReady: true,
    pubkey: 'pk',
    contexts: {},
    version: 0,
    forcedUnreadContexts: {
      'forced': 'forced',
      'forced-backlog': 'forced-backlog',
    },
  );

  test('the channel list bolds backlog and dots replies and mentions', () {
    final highlights = computeChannelListHighlights(
      channels: channels,
      readState: readState,
      channelsNotifier: notifier,
    );
    expect(highlights.unread, {
      'backlog',
      'both',
      'dm',
      'forced',
      'forced-backlog',
    });
    // A manual mark keeps real attention's dot without fabricating one.
    expect(highlights.attention, {'reply', 'mention', 'both', 'dm', 'forced'});
  });

  test('Solo highlights still bold for every unread event', () {
    expect(
      computeUnreadChannelIds(
        channels: channels,
        readState: readState,
        channelsNotifier: notifier,
      ),
      {'backlog', 'reply', 'mention', 'both', 'dm', 'forced', 'forced-backlog'},
    );
  });

  test('read marks clear both highlights', () {
    final highlights = computeChannelListHighlights(
      channels: channels,
      readState: const ReadStateState(
        isReady: true,
        pubkey: 'pk',
        contexts: {
          'backlog': 20,
          'reply': 20,
          'mention': 20,
          'both': 20,
          'dm': 20,
          'forced': 20,
          'forced-backlog': 20,
        },
        version: 0,
      ),
      channelsNotifier: notifier,
    );
    expect(highlights.unread, isEmpty);
    expect(highlights.attention, isEmpty);
  });
}
