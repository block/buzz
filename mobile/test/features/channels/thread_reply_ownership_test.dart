import 'package:buzz/features/channels/thread_reply_ownership.dart';
import 'package:buzz/shared/relay/relay.dart';
import 'package:flutter_test/flutter_test.dart';

void main() {
  test(
    'identity evidence is bounded while keeping recent evicted-payload owners',
    () {
      final owners = ThreadReplyOwnership();
      owners.record([
        for (var i = 0; i < 8193; i++)
          NostrEvent(
            id: 'reply-$i',
            pubkey: 'author',
            createdAt: i,
            kind: EventKind.streamMessage,
            tags: const [
              ['h', 'channel'],
              ['e', 'root', '', 'reply'],
            ],
            content: '',
            sig: '',
          ),
      ]);
      expect(owners.rootFor('reply-0'), isNull);
      expect(owners.rootFor('reply-1'), 'root');
      expect(owners.rootFor('reply-8192'), 'root');
    },
  );
}
