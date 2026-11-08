import 'package:buzz/shared/me/solo_chat_seed.dart';
import 'package:flutter_test/flutter_test.dart';

void main() {
  test('open channels link the source; closed ones quote it', () {
    for (final (linkable, root, content, seed) in [
      (true, null, 'ignored', 'buzz://message?channel=c1&id=m1\n\n'),
      (true, 'r1', 'ignored', 'buzz://message?channel=c1&id=m1&thread=r1\n\n'),
      (false, null, '  Ship it Friday  ', '> Ship it Friday\n\n'),
      (false, 'r1', 'line one\n\nline two', '> line one\n>\n> line two\n\n'),
    ]) {
      expect(
        soloChatSeed(
          linkable: linkable,
          channelId: 'c1',
          messageId: 'm1',
          threadRootId: root,
          content: content,
        ),
        seed,
      );
    }
  });
}
