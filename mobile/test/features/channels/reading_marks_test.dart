import 'package:buzz/features/channels/reading_marks.dart';
import 'package:buzz/features/channels/timeline_message.dart';
import 'package:buzz/shared/read_state/read_state_provider.dart';
import 'package:flutter_test/flutter_test.dart';

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
}) => readingMarks(
  readState: readState ?? _state(),
  channelId: _channel,
  isDm: isDm,
  currentPubkey: _self,
  loaded: loaded,
  visible: visible,
  bottom: bottom,
  threadRootId: threadRootId,
);

void main() {
  group('channel timeline', () {
    test('the bottom writes one catch-up mark instead of message marks', () {
      final a = _msg('a', 100);
      final b = _msg('b', 200);
      expect(_marks(loaded: [a, b], visible: [a, b], bottom: b), {
        'activity:$_channel': 200,
      });
    });

    test('catch-up time includes newer loaded replies', () {
      final a = _msg('a', 100);
      final reply = _msg('r', 300, parentId: 'a', rootId: 'a');
      expect(_marks(loaded: [a, reply], visible: [a], bottom: a), {
        'activity:$_channel': 300,
      });
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
        {'thread-activity:root': 200},
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
        {'thread-activity:root': 100},
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

    test('an existing thread mark already reads the replies', () {
      final r1 = reply('r1', 100);
      expect(
        _marks(
          readState: _state({'thread:root': 100}),
          loaded: [root, r1],
          visible: [r1],
          bottom: r1,
          threadRootId: 'root',
        ),
        isEmpty,
      );
    });
  });
}
