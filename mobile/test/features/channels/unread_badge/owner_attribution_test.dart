import 'package:buzz/features/channels/unread_badge/is_high_priority_event.dart';
import 'package:buzz/features/channels/unread_badge/should_notify_for_event.dart';
import 'package:buzz/shared/relay/nostr_models.dart';
import 'package:flutter_test/flutter_test.dart';

void main() {
  test('a workflow owner attribution neither notifies nor marks priority', () {
    final me = 'a' * 64;
    final attribution = ['p', me, '', 'owner'];
    for (final (tags, addressed) in [
      ([attribution], false),
      (
        [
          attribution,
          ['p', me],
        ],
        true,
      ),
    ]) {
      // A reply in a thread the reader hasn't joined notifies only when it
      // addresses them, so `addressed` is the whole outcome.
      final event = NostrEvent(
        id: 'b' * 64,
        pubkey: 'c' * 64,
        createdAt: 1_700_000_000,
        kind: 9,
        tags: [
          ...tags,
          ['buzz:workflow', 'true'],
          ['h', 'channel-1'],
          ['e', 'd' * 64, '', 'reply'],
        ],
        content: 'workflow output',
        sig: '',
      );
      expect(isHighPriorityEvent(event.tags, me), addressed, reason: '$tags');
      expect(shouldNotifyForEvent(event, me), addressed, reason: '$tags');
    }
  });
}
