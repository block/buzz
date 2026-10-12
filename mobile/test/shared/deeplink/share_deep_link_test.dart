import 'package:buzz/shared/deeplink/deep_link.dart';
import 'package:flutter_test/flutter_test.dart';

void main() {
  const id = '3f2504e0-4f89-41d3-9a0c-0305e82c3301';

  group('parseShareDeepLink', () {
    test('parses the canonical share hand-off', () {
      expect(
        parseShareDeepLink(Uri.parse('buzz://share?id=$id')),
        const ShareDeepLink(payloadId: id),
      );
    });

    test('is reachable through parseBuzzDeepLink', () {
      expect(
        parseBuzzDeepLink(Uri.parse('buzz://share?id=$id')),
        const ShareDeepLink(payloadId: id),
      );
    });

    test('rejects malformed or ambiguous forms', () {
      for (final url in [
        'buzz://share',
        'buzz://share?id=',
        'buzz://share?id=not-a-uuid',
        'buzz://share?id=${id.toUpperCase()}',
        'buzz://share?id=$id&extra=1',
        'buzz://share?id=$id&id=$id',
        'buzz://share/path?id=$id',
        'buzz://share?id=$id#fragment',
        'buzz://user@share?id=$id',
        'buzz://share:80?id=$id',
        'https://share?id=$id',
        'buzz://message?id=$id',
      ]) {
        expect(parseShareDeepLink(Uri.parse(url)), isNull, reason: url);
      }
    });
  });
}
