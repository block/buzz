import 'package:buzz/features/channels/thread_detail_page.dart';
import 'package:buzz/features/channels/timeline_message.dart';
import 'package:buzz/shared/profile/user_profile.dart';
import 'package:flutter_test/flutter_test.dart';

TimelineMessage _message({
  required String id,
  required String pubkey,
  required String content,
  bool isSystem = false,
}) => TimelineMessage(
  id: id,
  pubkey: pubkey,
  createdAt: 1786000000,
  content: content,
  isSystem: isSystem,
);

void main() {
  group('threadSummaryDigest', () {
    test('names authors the way the thread rows name them', () {
      final digest = threadSummaryDigest(
        [
          _message(id: '1', pubkey: 'ABC123', content: 'first'),
          _message(id: '2', pubkey: 'def456', content: 'second'),
        ],
        profiles: const {
          'abc123': UserProfile(pubkey: 'abc123', displayName: 'Ada'),
        },
      );

      expect(digest.map((entry) => entry.author), ['Ada', 'def456']);
      expect(digest.map((entry) => entry.text), ['first', 'second']);
    });

    test('shortens a long pubkey when no profile is cached', () {
      final digest = threadSummaryDigest([
        _message(
          id: '1',
          pubkey: 'aaaaaaaabbbbbbbbccccccccdddddddd',
          content: 'hello',
        ),
      ], profiles: const {});
      expect(digest.single.author, 'aaaaaaaa…');
    });

    test('drops system rows, which are chrome rather than conversation', () {
      final digest = threadSummaryDigest([
        _message(id: '1', pubkey: 'a', content: 'real message'),
        _message(id: '2', pubkey: 'a', content: 'joined', isSystem: true),
      ], profiles: const {});
      expect(digest.map((entry) => entry.text), ['real message']);
    });

    test('drops blank messages so they cannot pad the digest', () {
      final digest = threadSummaryDigest([
        _message(id: '1', pubkey: 'a', content: '   \n '),
        _message(id: '2', pubkey: 'a', content: 'kept'),
      ], profiles: const {});
      expect(digest.map((entry) => entry.text), ['kept']);
    });

    test('preserves thread order', () {
      final digest = threadSummaryDigest([
        _message(id: '1', pubkey: 'a', content: 'head'),
        _message(id: '2', pubkey: 'b', content: 'reply one'),
        _message(id: '3', pubkey: 'c', content: 'reply two'),
      ], profiles: const {});
      expect(digest.map((entry) => entry.text), [
        'head',
        'reply one',
        'reply two',
      ]);
    });

    test('yields an empty digest for a thread with nothing to say', () {
      expect(threadSummaryDigest(const [], profiles: const {}), isEmpty);
    });
  });
}
