import 'package:buzz/features/channels/thread_unread_marker.dart';
import 'package:buzz/features/channels/timeline_message.dart';
import 'package:flutter_test/flutter_test.dart';

const _me = 'aaaa';
const _other = 'bbbb';

TimelineMessage _reply(
  String id, {
  required int createdAt,
  String pubkey = _other,
}) => TimelineMessage(
  id: id,
  pubkey: pubkey,
  createdAt: createdAt,
  content: id,
  parentId: 'head',
  rootId: 'head',
);

String? _run(
  List<TimelineMessage> replies,
  Map<String, int?> snapshot, {
  Set<String> forced = const {},
  String? currentPubkey = _me,
}) => firstUnreadThreadReplyId(
  replies: replies,
  openReadSnapshot: snapshot,
  isForcedUnread: forced.contains,
  currentPubkey: currentPubkey,
);

void main() {
  group('firstUnreadThreadReplyId', () {
    test('returns null when every reply is read', () {
      final replies = [
        _reply('r1', createdAt: 100),
        _reply('r2', createdAt: 200),
      ];
      expect(_run(replies, {'r1': 100, 'r2': 200}), isNull);
    });

    test('returns the oldest reply newer than its frozen marker', () {
      final replies = [
        _reply('r1', createdAt: 100),
        _reply('r2', createdAt: 200),
        _reply('r3', createdAt: 300),
      ];
      expect(_run(replies, {'r1': 100, 'r2': 150, 'r3': 150}), 'r2');
    });

    test('a captured null means never read, not unknown', () {
      final replies = [_reply('r1', createdAt: 100)];
      expect(_run(replies, {'r1': null}), 'r1');
    });

    test('replies absent from the snapshot are skipped', () {
      // r1 was never observed at open time, so there is nothing to compare
      // against; r2 was observed and is stale.
      final replies = [
        _reply('r1', createdAt: 100),
        _reply('r2', createdAt: 200),
      ];
      expect(_run(replies, {'r2': 150}), 'r2');
    });

    test('own replies never count as unread', () {
      final replies = [
        _reply('mine', createdAt: 100, pubkey: _me),
        _reply('theirs', createdAt: 200),
      ];
      expect(_run(replies, {'mine': null, 'theirs': null}), 'theirs');
    });

    test('own-pubkey comparison is case insensitive', () {
      final replies = [_reply('mine', createdAt: 100, pubkey: 'AAAA')];
      expect(_run(replies, {'mine': null}), isNull);
    });

    test('every reply counts as unread when there is no local pubkey', () {
      final replies = [_reply('mine', createdAt: 100, pubkey: _me)];
      expect(_run(replies, {'mine': null}, currentPubkey: null), 'mine');
    });

    test('a forced-unread reply wins over an advanced marker', () {
      final replies = [
        _reply('r1', createdAt: 100),
        _reply('r2', createdAt: 200),
      ];
      expect(
        _run(replies, {'r1': 100, 'r2': 200}, forced: {'r2'}),
        'r2',
      );
    });

    test('a reply exactly at its marker is read (strict newer-than)', () {
      final replies = [_reply('r1', createdAt: 100)];
      expect(_run(replies, {'r1': 100}), isNull);
    });

    test('returns null for an empty thread', () {
      expect(_run(const [], const {}), isNull);
    });
  });
}
