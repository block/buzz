import 'dart:collection';
import 'dart:math';

import 'package:buzz/features/channels/channel.dart';
import 'package:buzz/features/channels/channel_unread.dart';
import 'package:buzz/features/channels/channels_provider.dart';
import 'package:buzz/features/channels/unread_badge/observed_unread_event.dart';
import 'package:buzz/shared/read_state/read_state_format.dart';
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

typedef _Highlights = ({
  Set<String> unread,
  Set<String> rowUnread,
  Set<String> attention,
});

ObservedUnreadEvent _eventAt(
  String id,
  int createdAt, {
  String type = 'stream',
  String? root,
  bool mention = false,
}) => makeObservedUnreadEvent(
  id: id,
  createdAt: createdAt,
  rootId: root,
  highPriority: mention,
  channelType: type,
  isThreadedReply: root != null,
);

/// The registry as the real notifier exposes it: a stamped snapshot, so
/// highlights may be reused. Counts every event a pass reads.
class _StampedChannels extends ChannelsNotifier {
  final Map<String, Map<String, ObservedUnreadEvent>> events = {};
  final revisions = ObservedUnreadRevisions();
  int visits = 0;

  void add(String channelId, ObservedUnreadEvent event) {
    (events[channelId] ??= {})[event.id] = event;
    revisions.touch(channelId);
  }

  void remove(String channelId, String eventId) {
    events[channelId]?.remove(eventId);
    if (events[channelId]?.isEmpty ?? false) events.remove(channelId);
    revisions.touch(channelId);
  }

  @override
  Map<String, int> get latestObservedByChannel => {
    for (final MapEntry(:key, :value) in events.entries)
      key: value.values.map((event) => event.createdAt).reduce(max),
  };

  @override
  Map<String, Map<String, ObservedUnreadEvent>>
  get observedUnreadEventsByChannel => ObservedUnreadSnapshot({
    for (final MapEntry(:key, :value) in events.entries)
      key: _CountingMap(value, () => visits++),
  }, revisions.of);
}

class _CountingMap extends MapView<String, ObservedUnreadEvent> {
  _CountingMap(super.map, this.onVisit);
  final void Function() onVisit;

  @override
  Iterable<ObservedUnreadEvent> get values => super.values.map((event) {
    onVisit();
    return event;
  });
}

/// The full-scan pass the memoized, early-exiting one must keep matching.
_Highlights _fullScan({
  required Iterable<Channel> channels,
  required ReadStateState readState,
  required Map<String, Map<String, ObservedUnreadEvent>> events,
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
  final forced = readState.locallyForcedChannelIds;
  for (final channel in channels) {
    if (forced.contains(channel.id)) {
      unread.add(channel.id);
      if (separateAttention) rowUnread.add(channel.id);
    }
    final observed = events[channel.id];
    if (observed == null) continue;
    final latest = observed.values.map((event) => event.createdAt).reduce(max);
    final channelReadAt = readState.effectiveTimestamp(channel.id);
    if (channelReadAt != null && latest <= channelReadAt) continue;
    final seenAt = separateAttention
        ? readState.contexts['seen:${channel.id}']
        : null;
    for (final event in observed.values) {
      final readAt = observedUnreadEventReadAt(
        event,
        channel.id,
        readState.effectiveTimestamp,
      );
      if (readAt != null && event.createdAt <= readAt) continue;
      if (!separateAttention || channel.isDm || event.channelCatchUp) {
        unread.add(channel.id);
        if (separateAttention &&
            (channel.isDm || seenAt == null || event.createdAt > seenAt)) {
          rowUnread.add(channel.id);
        }
      }
      if (separateAttention && (channel.isDm || !event.channelCatchUp)) {
        attention.add(channel.id);
      }
    }
  }
  return highlights;
}

void _expectMatchesFullScan(
  _StampedChannels notifier,
  List<Channel> channels,
  ReadStateState readState, {
  String? reason,
}) {
  final expected = _fullScan(
    channels: channels,
    readState: readState,
    events: notifier.events,
    separateAttention: true,
  );
  // Twice: a pass that reuses its previous outcome must agree with itself.
  for (var pass = 0; pass < 2; pass++) {
    final actual = computeChannelListHighlights(
      channels: channels,
      readState: readState,
      channelsNotifier: notifier,
    );
    expect(actual.unread, expected.unread, reason: reason);
    expect(actual.rowUnread, expected.rowUnread, reason: reason);
    expect(actual.attention, expected.attention, reason: reason);
    expect(
      computeUnreadChannelIds(
        channels: channels,
        readState: readState,
        channelsNotifier: notifier,
      ),
      _fullScan(
        channels: channels,
        readState: readState,
        events: notifier.events,
        separateAttention: false,
      ).unread,
      reason: reason,
    );
  }
}

ReadStateState _reads(
  Map<String, int> contexts, {
  bool isReady = true,
  Set<String> forced = const {},
}) => ReadStateState(
  isReady: isReady,
  pubkey: 'pk',
  contexts: contexts,
  version: 0,
  forcedUnreadContexts: {for (final id in forced) id: id},
);

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
  group('memoized, early-exiting pass', () {
    final listed = [
      _channel('stream'),
      _channel('dm', type: 'dm'),
      _channel('quiet'),
    ];
    _StampedChannels seeded() => _StampedChannels()
      ..add('stream', _eventAt('m1', 10))
      ..add('stream', _eventAt('r1', 12, root: 'root'))
      ..add('stream', _eventAt('p1', 14, mention: true))
      ..add('stream', _eventAt('m2', 16))
      ..add('dm', _eventAt('d1', 10, type: 'dm'))
      ..add('dm', _eventAt('d2', 16, type: 'dm', root: 'dm-root'));

    final cases = <String, ReadStateState>{
      'nothing read': _reads({}),
      'read state not ready': _reads({}, isReady: false),
      'channel marker covers everything': _reads({'stream': 16, 'dm': 16}),
      'channel marker covers part': _reads({'stream': 13, 'dm': 12}),
      'catch-up read, attention left': _reads({
        activityContextKey('stream'): 16,
      }),
      'attention read, catch-up left': _reads({
        threadContextKey('root'): 12,
        msgContextKey('p1'): 14,
      }),
      'thread activity mark reads the reply': _reads({
        threadActivityContextKey('root'): 12,
      }),
      'every event read by its own mark': _reads({
        msgContextKey('m1'): 10,
        msgContextKey('r1'): 12,
        msgContextKey('p1'): 14,
        msgContextKey('m2'): 16,
        msgContextKey('d1'): 10,
        msgContextKey('d2'): 16,
      }),
      'seen mark covers the backlog': _reads({
        'seen:stream': 16,
        'seen:dm': 16,
      }),
      'seen mark covers part of the backlog': _reads({'seen:stream': 11}),
      'forced with evidence': _reads({}, forced: {'stream', 'dm'}),
      'forced over a read channel': _reads(
        {'stream': 16, 'dm': 16},
        forced: {'stream', 'dm'},
      ),
      'forced with nothing observed': _reads({}, forced: {'quiet'}),
      'forced, attention read': _reads(
        {threadContextKey('root'): 12, msgContextKey('p1'): 14},
        forced: {'stream'},
      ),
    };

    for (final MapEntry(key: name, value: readState) in cases.entries) {
      test('matches the full scan: $name', () {
        _expectMatchesFullScan(seeded(), listed, readState);
      });
    }

    test('matches the full scan as events, reads and channels change', () {
      final random = Random(7);
      final all = [
        for (var i = 0; i < 6; i++)
          _channel('c$i', type: i.isEven ? 'stream' : 'dm'),
      ];
      final notifier = _StampedChannels();
      var contexts = <String, int>{};
      var forced = <String>{};
      var readState = _reads(contexts);
      var listed = all;
      for (var step = 0; step < 600; step++) {
        final channel = all[random.nextInt(all.length)];
        final at = 1 + random.nextInt(30);
        switch (random.nextInt(8)) {
          case 0 || 1 || 2:
            final kind = random.nextInt(3);
            notifier.add(
              channel.id,
              _eventAt(
                'e$step',
                at,
                type: channel.channelType,
                root: kind == 1 ? 'root${random.nextInt(3)}' : null,
                mention: kind == 2,
              ),
            );
          case 3:
            final ids = notifier.events[channel.id]?.keys.toList() ?? [];
            if (ids.isNotEmpty) {
              notifier.remove(channel.id, ids[random.nextInt(ids.length)]);
            }
          case 4:
            final ids = notifier.events[channel.id]?.keys.toList() ?? [];
            final key = [
              channel.id,
              'seen:${channel.id}',
              activityContextKey(channel.id),
              threadContextKey('root${random.nextInt(3)}'),
              threadActivityContextKey('root${random.nextInt(3)}'),
              if (ids.isNotEmpty)
                msgContextKey(ids[random.nextInt(ids.length)]),
            ];
            contexts = {...contexts, key[random.nextInt(key.length)]: at};
            readState = _reads(contexts, forced: forced);
          case 5:
            forced = forced.contains(channel.id)
                ? forced.difference({channel.id})
                : {...forced, channel.id};
            readState = _reads(contexts, forced: forced);
          case 6:
            listed = [
              for (final candidate in all)
                if (random.nextBool()) candidate,
            ];
          case 7:
            readState = _reads(
              contexts,
              forced: forced,
              isReady: random.nextInt(4) != 0,
            );
        }
        _expectMatchesFullScan(
          notifier,
          listed,
          readState,
          reason: 'step $step',
        );
      }
    });

    test('a channel type change is not served from the old outcome', () {
      final notifier = _StampedChannels()..add('c', _eventAt('m', 10));
      final readState = _reads({});
      _expectMatchesFullScan(notifier, [_channel('c')], readState);
      _expectMatchesFullScan(notifier, [_channel('c', type: 'dm')], readState);
    });

    test('a rebuild reads no events until its inputs change', () {
      const backlog = 1000;
      final listed = [for (var i = 0; i < 20; i++) _channel('c$i')];
      final notifier = _StampedChannels();
      for (final channel in listed) {
        for (var i = 0; i < backlog; i++) {
          notifier.add(channel.id, _eventAt('${channel.id}-$i', 10 + i));
        }
      }
      var readState = _reads({});
      int pass() {
        notifier.visits = 0;
        final highlights = computeChannelListHighlights(
          channels: listed,
          readState: readState,
          channelsNotifier: notifier,
        );
        expect(highlights.rowUnread, {for (final c in listed) c.id});
        return notifier.visits;
      }

      // Backlog alone never decides attention, so the first pass reads it all.
      expect(pass(), listed.length * backlog);
      expect(pass(), 0, reason: 'unchanged inputs');
      expect(pass(), 0, reason: 'unchanged inputs, again');

      // The list order and membership are not part of a channel's outcome.
      listed.insert(0, listed.removeLast());
      expect(pass(), 0, reason: 'reordered list');

      notifier.add('c3', _eventAt('late', 5000));
      expect(pass(), backlog + 1, reason: 'only the changed channel');

      notifier.remove('c3', 'late');
      expect(pass(), backlog, reason: 'a deletion invalidates its channel');

      readState = _reads({'c0': 10});
      expect(pass(), listed.length * backlog, reason: 'new read state');
      expect(pass(), 0);
    });

    test('a channel stops being read once its outcome is decided', () {
      const backlog = 1000;
      final notifier = _StampedChannels();
      for (final id in ['stream', 'forced']) {
        notifier
          ..add(id, _eventAt('$id-m', 10))
          ..add(id, _eventAt('$id-p', 11, mention: true));
      }
      notifier.add('dm', _eventAt('dm-0', 10, type: 'dm'));
      for (final id in ['stream', 'forced', 'dm']) {
        for (var i = 0; i < backlog; i++) {
          notifier.add(
            id,
            _eventAt('$id-b$i', 20 + i, type: id == 'dm' ? 'dm' : 'stream'),
          );
        }
      }
      final listed = [
        _channel('stream'),
        _channel('forced'),
        _channel('dm', type: 'dm'),
      ];
      final readState = _reads({}, forced: {'forced'});

      final highlights = computeChannelListHighlights(
        channels: listed,
        readState: readState,
        channelsNotifier: notifier,
      );
      expect(highlights.attention, {'stream', 'forced', 'dm'});
      // Backlog then mention decides a stream; a DM's first event decides it.
      expect(notifier.visits, 2 + 2 + 1);

      notifier.visits = 0;
      computeUnreadChannelIds(
        channels: listed,
        readState: readState,
        channelsNotifier: notifier,
      );
      // One unread event bolds; a forced channel needs none.
      expect(notifier.visits, 1 + 0 + 1);
    });
  });
}
