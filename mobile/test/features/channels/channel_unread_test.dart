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

  group('row hints (seen:)', () {
    ReadStateState reads(
      Map<String, int> contexts, {
      Map<String, String> forced = const {},
      int retentionFloor = 0,
    }) => ReadStateState(
      isReady: true,
      pubkey: 'pk',
      contexts: contexts,
      version: 0,
      forcedUnreadContexts: forced,
      retentionFloor: retentionFloor,
    );
    final rows = channels.where((c) => !c.id.startsWith('forced')).toList();

    test('opening saves hints that quiet the row and read nothing', () {
      final marks = <String, int>{};
      for (final channel in rows) {
        marks.addAll(
          channelRowSeenMarks(
            channelId: channel.id,
            events: notifier.observedUnreadEventsByChannel[channel.id]!,
            readState: reads(const {}),
          ),
        );
      }
      expect(marks, {
        'seen:backlog': 20,
        'seen:msg:r1': 20,
        'seen:msg:p1': 20,
        'seen:both': 20,
        'seen:msg:r2': 20,
        'seen:msg:d1': 20,
      });
      final seen = reads(marks);
      final highlights = computeChannelListHighlights(
        channels: rows,
        readState: seen,
        channelsNotifier: notifier,
      );
      expect(highlights.unread, isEmpty);
      expect(highlights.attention, isEmpty);
      // Nothing was read: Solo and every other reader still see it unread.
      expect(
        computeUnreadChannelIds(
          channels: rows,
          readState: seen,
          channelsNotifier: notifier,
        ),
        {for (final channel in rows) channel.id},
      );
      // A revisit with nothing new writes nothing.
      for (final channel in rows) {
        expect(
          channelRowSeenMarks(
            channelId: channel.id,
            events: notifier.observedUnreadEventsByChannel[channel.id]!,
            readState: seen,
          ),
          isEmpty,
        );
      }
    });

    test('read events need no hint', () {
      expect(
        channelRowSeenMarks(
          channelId: 'both',
          events: notifier.observedUnreadEventsByChannel['both']!,
          readState: reads({'both': 20}),
        ),
        isEmpty,
      );
    });

    test('attention lights the row again unless its own receipt saw it', () {
      final highlights = computeChannelListHighlights(
        channels: rows,
        readState: reads({
          'seen:backlog': 19,
          'seen:reply': 99,
          'seen:mention': 99,
          'seen:msg:other': 99,
        }),
        channelsNotifier: notifier,
      );
      // An ordinary message newer than the hint, and attention the channel
      // hint cannot cover, whatever its time.
      expect(highlights.unread, {'backlog', 'both', 'dm'});
      expect(highlights.attention, {'reply', 'mention', 'both', 'dm'});
    });

    test('the retention floor does not stand in for a receipt', () {
      final highlights = computeChannelListHighlights(
        channels: [_channel('reply')],
        readState: reads(const {}, retentionFloor: 10),
        channelsNotifier: notifier,
      );
      expect(highlights.attention, {'reply'});
    });

    test('a manual unread shows the whole row again', () {
      final highlights = computeChannelListHighlights(
        channels: channels,
        readState: reads(
          {'seen:forced-backlog': 20, 'seen:msg:p2': 20},
          forced: {'forced': 'forced', 'forced-backlog': 'forced-backlog'},
        ),
        channelsNotifier: notifier,
      );
      expect(highlights.unread, containsAll(['forced', 'forced-backlog']));
      expect(highlights.attention, contains('forced'));
    });
  });
}
