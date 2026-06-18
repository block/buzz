import 'package:flutter_test/flutter_test.dart';
import 'package:buzz/features/channels/message_media.dart';

void main() {
  group('classifyMediaUrl', () {
    test('treats only mp4 URLs as video fallback', () {
      expect(
        classifyMediaUrl('https://example.com/media/clip.mp4'),
        MessageMediaKind.video,
      );
      expect(classifyMediaUrl('https://example.com/media/clip.mov'), isNull);
      expect(classifyMediaUrl('https://example.com/media/clip.webm'), isNull);
    });

    test('does not treat non-mp4 video mimetypes as video UI', () {
      expect(
        classifyMediaUrl(
          'https://example.com/media/clip.mov',
          imeta: const ImetaEntry(
            url: 'https://example.com/media/clip.mov',
            mimeType: 'video/quicktime',
          ),
        ),
        isNull,
      );
      expect(
        classifyMediaUrl(
          'https://example.com/media/clip.mp4',
          imeta: const ImetaEntry(
            url: 'https://example.com/media/clip.mp4',
            mimeType: 'video/mp4',
          ),
        ),
        MessageMediaKind.video,
      );
    });
  });
}
