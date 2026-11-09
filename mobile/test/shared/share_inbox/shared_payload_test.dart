import 'package:buzz/shared/share_inbox/shared_payload.dart';
import 'package:flutter_test/flutter_test.dart';

void main() {
  group('SharedPayload.fromMap', () {
    test('parses text, url, and file items', () {
      final payload = SharedPayload.fromMap({
        'id': 'abc',
        'createdAt': 1.0,
        'items': [
          {'kind': 'text', 'value': 'hello'},
          {'kind': 'url', 'value': 'https://example.com'},
          {
            'kind': 'file',
            'path': '/tmp/a.png',
            'name': 'a.png',
            'mimeType': 'image/png',
          },
        ],
      });
      expect(payload, isNotNull);
      expect(payload!.id, 'abc');
      expect(payload.items, hasLength(3));
      expect(payload.composedText, 'hello\nhttps://example.com');
      expect(payload.files, [
        const SharedItem(
          kind: SharedItemKind.file,
          path: '/tmp/a.png',
          name: 'a.png',
          mimeType: 'image/png',
        ),
      ]);
    });

    test('carries the destination picked in the extension', () {
      final payload = SharedPayload.fromMap({
        'id': 'abc',
        'items': [
          {'kind': 'text', 'value': 'hello'},
        ],
        'communityId': 'c1',
        'channelId': 'ch1',
      });
      expect(payload!.hasTarget, isTrue);
      expect(payload.communityId, 'c1');
      expect(payload.channelId, 'ch1');

      final untargeted = SharedPayload.fromMap({
        'id': 'abc',
        'items': [
          {'kind': 'text', 'value': 'hello'},
        ],
        'communityId': '',
      });
      expect(untargeted!.hasTarget, isFalse);
    });

    test('drops unusable items and rejects empty payloads', () {
      final payload = SharedPayload.fromMap({
        'id': 'abc',
        'items': [
          {'kind': 'text', 'value': '   '},
          {'kind': 'file', 'path': ''},
          {'kind': 'mystery', 'value': 'x'},
          {'kind': 'url', 'value': 'https://example.com'},
        ],
      });
      expect(payload!.items, hasLength(1));
      expect(payload.composedText, 'https://example.com');

      expect(
        SharedPayload.fromMap({
          'id': 'abc',
          'items': [
            {'kind': 'text', 'value': ''},
          ],
        }),
        isNull,
      );
      expect(SharedPayload.fromMap({'items': []}), isNull);
      expect(SharedPayload.fromMap(null), isNull);
    });
  });
}
