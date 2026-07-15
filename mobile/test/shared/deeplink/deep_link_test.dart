import 'package:buzz/shared/deeplink/deep_link.dart';
import 'package:flutter_test/flutter_test.dart';

void main() {
  group('parseMessageDeepLink', () {
    test('parses channel and id', () {
      final link = parseMessageDeepLink(
        Uri.parse('buzz://message?channel=d14cd131&id=abc123'),
      );
      expect(
        link,
        const MessageDeepLink(channelId: 'd14cd131', messageId: 'abc123'),
      );
    });

    test('parses optional thread param', () {
      final link = parseMessageDeepLink(
        Uri.parse('buzz://message?channel=d14cd131&id=abc123&thread=root99'),
      );
      expect(link?.threadRootId, 'root99');
    });

    test('treats empty thread as absent', () {
      final link = parseMessageDeepLink(
        Uri.parse('buzz://message?channel=d14cd131&id=abc123&thread='),
      );
      expect(link, isNotNull);
      expect(link?.threadRootId, isNull);
    });

    test('rejects missing channel', () {
      expect(parseMessageDeepLink(Uri.parse('buzz://message?id=abc')), isNull);
    });

    test('rejects empty channel', () {
      expect(
        parseMessageDeepLink(Uri.parse('buzz://message?channel=&id=abc')),
        isNull,
      );
    });

    test('rejects missing id', () {
      expect(
        parseMessageDeepLink(Uri.parse('buzz://message?channel=d14cd131')),
        isNull,
      );
    });

    test('rejects non-buzz scheme', () {
      expect(
        parseMessageDeepLink(Uri.parse('https://message?channel=a&id=b')),
        isNull,
      );
    });

    test('rejects non-message host (connect is desktop-only)', () {
      expect(
        parseMessageDeepLink(Uri.parse('buzz://connect?relay=wss://x')),
        isNull,
      );
    });
  });
}
