import 'package:buzz/features/channels/reading_marks.dart';
import 'package:buzz/features/channels/timeline_message.dart';
import 'package:buzz/shared/read_state/read_state_provider.dart';
import 'package:flutter/widgets.dart';
import 'package:flutter_hooks/flutter_hooks.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:scrollable_positioned_list/scrollable_positioned_list.dart';

const _channel = 'channel-1';
const _self = 'self';

TimelineMessage _msg(
  String id,
  int createdAt, {
  String pubkey = 'alice',
  String? parentId,
  String? rootId,
  List<List<String>> tags = const [],
}) => TimelineMessage(
  id: id,
  pubkey: pubkey,
  createdAt: createdAt,
  content: id,
  tags: tags,
  parentId: parentId,
  rootId: rootId,
);

ReadStateState _state([
  Map<String, int> contexts = const {},
  Map<String, String> forced = const {},
]) => ReadStateState(
  isReady: true,
  pubkey: _self,
  contexts: contexts,
  version: 0,
  forcedUnreadContexts: forced,
);

Map<String, int> _marks({
  ReadStateState? readState,
  bool isDm = false,
  required List<TimelineMessage> loaded,
  List<TimelineMessage> visible = const [],
  TimelineMessage? bottom,
  String? threadRootId,
  bool isRootThread = true,
  int now = 10000,
}) => readingMarks(
  readState: readState ?? _state(),
  channelId: _channel,
  isDm: isDm,
  currentPubkey: _self,
  loaded: loaded,
  visible: visible,
  bottom: bottom,
  threadRootId: threadRootId,
  isRootThread: isRootThread,
  now: now,
);

void main() {
  test('hidden endings never advance catch-up past the visible row', () {
    final bottom = _msg('visible', 100);
    final end = TimelineMessage(
      id: 'end',
      pubkey: 'alice',
      createdAt: 150,
      content: '',
      systemEvent: const SystemEvent(
        type: SystemEventType.huddleEnded,
        ephemeralChannelId: 'room',
      ),
    );
    Map<String, int> marks(List<TimelineMessage> loaded, int now) =>
        readingMarks(
          readState: _state(),
          channelId: _channel,
          isDm: false,
          currentPubkey: _self,
          loaded: [
            TimelineMessage(
              id: 'start',
              pubkey: 'alice',
              createdAt: 50,
              content: '',
              systemEvent: const SystemEvent(
                type: SystemEventType.huddleStarted,
                ephemeralChannelId: 'room',
              ),
            ),
            ...loaded,
          ],
          visible: [],
          bottom: bottom,
          now: now,
        );
    expect(marks([bottom, end], 200)['activity:$_channel'], 100);
    expect(marks([bottom, end], 125)['activity:$_channel'], 100);
    expect(marks([bottom, end], 90)['activity:$_channel'], 90);
    expect(marks([bottom, _msg('unseen', 120), end], 200), isEmpty);
    final late = _msg('late', 120);
    expect(
      _marks(
        readState: _state(marks([bottom, end], 200)),
        loaded: [bottom, late, end],
        visible: [late],
      ),
      {'msg:late': 120},
    );
  });

  test('hidden membership notices do not block catch-up', () {
    final bottom = _msg('visible', 100);
    TimelineMessage notice(String id, SystemEvent event) => TimelineMessage(
      id: id,
      pubkey: 'alice',
      createdAt: 150,
      content: '',
      isSystem: true,
      systemEvent: event,
    );
    for (final hidden in [
      notice(
        'join',
        const SystemEvent(
          type: SystemEventType.memberJoined,
          actorPubkey: 'alice',
          targetPubkey: 'alice',
        ),
      ),
      notice(
        'leave',
        const SystemEvent(
          type: SystemEventType.memberLeft,
          actorPubkey: 'alice',
        ),
      ),
    ]) {
      expect(
        _marks(loaded: [bottom, hidden], visible: [bottom], bottom: bottom),
        {'activity:$_channel': 100},
      );
    }
  });

  group('channel timeline', () {
    test('the bottom writes one catch-up mark instead of message marks', () {
      final a = _msg('a', 100);
      final b = _msg('b', 200);
      expect(_marks(loaded: [a, b], visible: [a, b], bottom: b), {
        'activity:$_channel': 200,
      });
    });

    test('catch-up cuts at the bottom row, not a newer reply', () {
      // A top-level message created at 200 that arrives late must stay
      // unread, so the cut cannot move up to the reply at 300.
      final a = _msg('a', 100);
      final reply = _msg('r', 300, parentId: 'a', rootId: 'a');
      expect(_marks(loaded: [a, reply], visible: [a], bottom: a), {
        'activity:$_channel': 100,
      });
    });

    test('catch-up never passes the current time', () {
      // A sender clock ahead of this one must not read messages that have
      // not arrived yet.
      final future = _msg('f', 20000);
      expect(
        _marks(loaded: [future], visible: [future], bottom: future, now: 10000),
        {'activity:$_channel': 10000, 'msg:f': 20000},
      );
    });

    test('a newer top-level message means the bottom is not live', () {
      final a = _msg('a', 100);
      final b = _msg('b', 200);
      expect(_marks(loaded: [a, b], visible: [a], bottom: a), {'msg:a': 100});
    });

    test('a newer broadcast reply means the bottom is not live', () {
      final a = _msg('a', 100);
      final broadcast = _msg(
        'r',
        300,
        parentId: 'a',
        rootId: 'a',
        tags: const [
          ['broadcast', '1'],
        ],
      );
      expect(_marks(loaded: [a, broadcast], visible: [a], bottom: a), {
        'msg:a': 100,
      });
    });

    test('mentions and broadcasts keep their own marks at the bottom', () {
      final mention = _msg(
        'm',
        100,
        tags: const [
          ['p', _self],
        ],
      );
      final b = _msg('b', 200);
      expect(_marks(loaded: [mention, b], visible: [mention, b], bottom: b), {
        'activity:$_channel': 200,
        'msg:m': 100,
      });
    });

    test('the bottom does not read mentions above it', () {
      // Only a mention that showed is read. Reaching the bottom past one that
      // never filled the screen leaves it unread.
      final mention = _msg(
        'm',
        100,
        tags: const [
          ['p', _self],
        ],
      );
      final broadcast = _msg(
        'bc',
        110,
        parentId: 'root',
        rootId: 'root',
        tags: const [
          ['broadcast', '1'],
          ['p', _self],
        ],
      );
      final b = _msg('b', 200);
      expect(_marks(loaded: [mention, broadcast, b], visible: [b], bottom: b), {
        'activity:$_channel': 200,
      });
    });

    test('the newest message is read at the bottom even when not visible', () {
      // A mention taller than the screen shows only its end at the bottom.
      final mention = _msg(
        'm',
        200,
        tags: const [
          ['p', _self],
        ],
      );
      expect(
        _marks(
          readState: _state(const {}, {'msg:forced': _channel}),
          loaded: [_msg('a', 100), mention],
          bottom: mention,
        ),
        {'activity:$_channel': 200, 'msg:m': 200},
      );
      final forced = _msg(
        'forced',
        200,
        tags: const [
          ['p', _self],
        ],
      );
      expect(
        _marks(
          readState: _state(const {}, {'msg:forced': _channel}),
          loaded: [forced],
          bottom: forced,
        ),
        {'activity:$_channel': 200},
      );
    });

    test('catch-up does not mark a mention away from the live bottom', () {
      final mention = _msg(
        'm',
        100,
        tags: const [
          ['p', _self],
        ],
      );
      final a = _msg('a', 150);
      final b = _msg('b', 200);
      expect(_marks(loaded: [mention, a, b], visible: [a], bottom: a), {
        'msg:a': 150,
      });
    });

    test('a flat timeline reads only its newest inline reply', () {
      final reply = _msg(
        'r',
        120,
        parentId: 'root',
        rootId: 'root',
        tags: const [
          ['p', _self],
        ],
      );
      final b = _msg('b', 200, parentId: 'root', rootId: 'root');
      Map<String, int> marks(TimelineMessage bottom) => readingMarks(
        readState: _state(),
        channelId: _channel,
        isDm: false,
        currentPubkey: _self,
        loaded: [reply, b],
        visible: const [],
        bottom: bottom,
        flat: true,
        now: 10000,
      );
      expect(marks(b), {'activity:$_channel': 200, 'msg:b': 200});
      // A newer inline reply means the bottom is not live.
      expect(marks(reply), isEmpty);
    });

    test('away from the bottom, visible unread rows get message marks', () {
      final a = _msg('a', 100);
      final b = _msg('b', 200);
      final c = _msg('c', 300);
      expect(
        _marks(
          readState: _state({'activity:$_channel': 150}),
          loaded: [a, b, c],
          visible: [a, b],
        ),
        {'msg:b': 200},
      );
    });

    test('never writes the channel mark', () {
      final a = _msg('a', 100);
      final marks = _marks(loaded: [a], visible: [a], bottom: a);
      expect(marks.containsKey(_channel), isFalse);
    });

    test('writes nothing when existing marks already read everything', () {
      final a = _msg('a', 100);
      expect(
        _marks(
          readState: _state({_channel: 100}),
          loaded: [a],
          visible: [a],
          bottom: a,
        ),
        isEmpty,
      );
      expect(
        _marks(
          readState: _state({'activity:$_channel': 100}),
          loaded: [a],
          visible: [a],
          bottom: a,
        ),
        isEmpty,
      );
    });

    test('skips own, system and manually unread messages', () {
      final own = _msg('own', 100, pubkey: _self);
      final system = TimelineMessage(
        id: 'sys',
        pubkey: 'alice',
        createdAt: 110,
        content: '',
        isSystem: true,
      );
      final forced = _msg('forced', 120);
      expect(
        _marks(
          readState: _state(const {}, {'msg:forced': _channel}),
          loaded: [own, system, forced, _msg('later', 200)],
          visible: [own, system, forced],
        ),
        isEmpty,
      );
    });

    test('in a DM, catch-up does not mark mentions above the bottom', () {
      final mention = _msg(
        'mention',
        100,
        tags: const [
          ['p', _self],
        ],
      );
      final b = _msg('b', 200);
      expect(_marks(isDm: true, loaded: [mention, b], bottom: b), {
        'activity:$_channel': 200,
        'msg:b': 200,
      });
    });

    test('in a DM, catch-up does not read messages', () {
      final a = _msg('a', 100);
      final b = _msg('b', 200);
      expect(_marks(isDm: true, loaded: [a, b], visible: [a, b], bottom: b), {
        'activity:$_channel': 200,
        'msg:a': 100,
        'msg:b': 200,
      });
    });
  });

  group('isReadableRow', () {
    bool readable(double leading, double trailing) => isReadableRow(
      ItemPosition(
        index: 0,
        itemLeadingEdge: leading,
        itemTrailingEdge: trailing,
      ),
      start: 0.1,
      end: 0.9,
    );

    test('reads a row inside the reading area', () {
      expect(readable(0.2, 0.8), isTrue);
      expect(readable(0.1, 0.9), isTrue);
    });

    test('reads a row that covers the whole reading area', () {
      expect(readable(-0.5, 1.4), isTrue);
      expect(readable(0.1, 1.4), isTrue);
    });

    test('does not read a row cut off at one edge', () {
      expect(readable(0.05, 0.5), isFalse);
      expect(readable(0.5, 0.95), isFalse);
      expect(readable(0.5, 2), isFalse);
      expect(readable(-1, 0.5), isFalse);
    });

    test('reads a linked row once any part is inside the reading area', () {
      bool linked(double leading, double trailing) => isReadableRow(
        ItemPosition(
          index: 0,
          itemLeadingEdge: leading,
          itemTrailingEdge: trailing,
        ),
        start: 0.1,
        end: 0.9,
        linked: true,
      );
      expect(linked(0.35, 2), isTrue);
      expect(linked(-1, 0.5), isTrue);
      expect(linked(0.2, 0.8), isTrue);
      expect(linked(-1, 0.1), isFalse);
      expect(linked(0.9, 2), isFalse);
    });
  });

  group('thread', () {
    final root = _msg('root', 50);
    TimelineMessage reply(String id, int at, {List<List<String>>? tags}) =>
        _msg(id, at, parentId: 'root', rootId: 'root', tags: tags ?? const []);

    test('the tail writes one thread catch-up mark', () {
      final r1 = reply('r1', 100);
      final mention = reply(
        'r2',
        200,
        tags: const [
          ['p', _self],
        ],
      );
      expect(
        _marks(
          loaded: [root, r1, mention],
          visible: [r1, mention],
          bottom: mention,
          threadRootId: 'root',
        ),
        // Thread catch-up reads a reply only while its root is loaded, so
        // each visible reply keeps its own mark, as in `buzz-app`.
        {'thread-activity:root': 200, 'msg:r1': 100, 'msg:r2': 200},
      );
    });

    test('thread catch-up uses the newest reply, not newer channel rows', () {
      final r1 = reply('r1', 100);
      final later = _msg('later', 400);
      expect(
        _marks(
          loaded: [root, r1, later],
          visible: [r1],
          bottom: r1,
          threadRootId: 'root',
        ),
        {'thread-activity:root': 100, 'msg:r1': 100},
      );
    });

    test('a newer reply in another branch means the tail is not live', () {
      final r1 = reply('r1', 100);
      final nested = _msg('n1', 300, parentId: 'other', rootId: 'root');
      expect(
        _marks(
          loaded: [root, r1, nested],
          visible: [r1],
          bottom: r1,
          threadRootId: 'root',
        ),
        {'msg:r1': 100},
      );
    });

    test('away from the tail, visible replies get message marks', () {
      final r1 = reply('r1', 100);
      final r2 = reply('r2', 200);
      expect(
        _marks(loaded: [root, r1, r2], visible: [r1], threadRootId: 'root'),
        {'msg:r1': 100},
      );
    });

    test('the tail reply is read at its end even when it never fits', () {
      // A reply taller than the screen shows only its end at the tail, so it
      // is never in [visible].
      final mention = reply(
        'r1',
        100,
        tags: const [
          ['p', _self],
        ],
      );
      expect(
        _marks(loaded: [root, mention], bottom: mention, threadRootId: 'root'),
        {'thread-activity:root': 100, 'msg:r1': 100},
      );
      // Away from the live tail it waits until it fills the screen.
      expect(
        _marks(
          loaded: [root, mention, reply('r2', 200)],
          bottom: mention,
          threadRootId: 'root',
        ),
        isEmpty,
      );
    });

    test('a thread mark replaces catch-up but not reply marks', () {
      final r1 = reply('r1', 100);
      expect(
        _marks(
          readState: _state({'thread:root': 100}),
          loaded: [root, r1],
          visible: [r1],
          bottom: r1,
          threadRootId: 'root',
        ),
        {'msg:r1': 100},
      );
    });

    test('a nested thread head does not catch up the whole thread', () {
      // Opened on a nested reply, the page shows one branch. An older
      // mention in another branch must stay unread.
      final branch = _msg('b1', 100, parentId: 'root', rootId: 'root');
      final nested = _msg('n1', 200, parentId: 'b1', rootId: 'root');
      expect(
        _marks(
          loaded: [root, branch, nested],
          visible: [nested],
          bottom: nested,
          threadRootId: 'root',
          isRootThread: false,
        ),
        {'msg:n1': 200},
      );
    });
  });

  group('readingContentKey', () {
    test('ignores a rebuilt list with the same messages', () {
      final a = _msg('a', 100);
      final b = _msg('b', 200);
      expect(readingContentKey([a, b]), readingContentKey([a, b].toList()));
      expect(readingContentKey([a, b]), isNot(readingContentKey([a])));
      expect(
        readingContentKey([a, b]),
        isNot(readingContentKey([a, _msg('c', 300)])),
      );
    });
  });

  testWidgets('the dwell fires while the page rebuilds every 200 ms', (
    tester,
  ) async {
    // A typing indicator or stream rebuilds the page with a new message
    // list. The dwell must still finish while the messages stay the same.
    final positions = ValueNotifier<Object?>(null);
    addTearDown(positions.dispose);
    final tick = ValueNotifier<int>(0);
    addTearDown(tick.dispose);
    final a = _msg('a', 100);
    var fired = 0;
    await tester.pumpWidget(
      ValueListenableBuilder<int>(
        valueListenable: tick,
        builder: (_, _, _) => HookBuilder(
          builder: (_) {
            final messages = [a];
            useReadingDwell(
              positions: positions,
              active: true,
              onDwell: () => fired++,
              keys: [readingContentKey(messages)],
            );
            return const SizedBox();
          },
        ),
      ),
    );
    // Rebuild at 200, 400 and 600 ms. A restarted dwell would not finish.
    for (var i = 0; i < 3; i++) {
      await tester.pump(const Duration(milliseconds: 200));
      tick.value++;
      await tester.pump();
    }
    expect(fired, 1);
  });
}
