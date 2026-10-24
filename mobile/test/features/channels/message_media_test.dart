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

    test('uses explicit video mimetypes for the video UI', () {
      expect(
        classifyMediaUrl(
          'https://example.com/media/clip.mov',
          imeta: const ImetaEntry(
            url: 'https://example.com/media/clip.mov',
            mimeType: 'video/quicktime',
          ),
        ),
        MessageMediaKind.video,
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

    test('classifies voice notes from imeta or an audio extension', () {
      expect(
        classifyMediaUrl(
          'https://example.com/media/blob',
          imeta: const ImetaEntry(
            url: 'https://example.com/media/blob',
            mimeType: 'audio/mp4',
          ),
        ),
        MessageMediaKind.audio,
      );
      expect(
        classifyMediaUrl('https://example.com/media/voice-note.m4a'),
        MessageMediaKind.audio,
      );
      expect(
        classifyMediaUrl(
          'https://example.com/media/blob.mp4',
          imeta: const ImetaEntry(
            url: 'https://example.com/media/blob.mp4',
            mimeType: 'video/mp4',
            filename: 'voice-note-test.mp4',
          ),
        ),
        MessageMediaKind.audio,
      );
    });

    test('rejects non-finite and negative durations', () {
      for (final duration in ['NaN', 'Infinity', '-1']) {
        final entry = parseImetaTags([
          [
            'imeta',
            'url https://example.com/media/$duration',
            'duration $duration',
          ],
        ]).values.single;
        expect(entry.duration, isNull);
      }
    });

    test('parses voice note duration and filename metadata', () {
      final entry = parseImetaTags(const [
        [
          'imeta',
          'url https://example.com/media/voice-note.m4a',
          'm audio/mp4',
          'duration 3.25',
          'filename voice-note.m4a',
          'size 42',
        ],
      ]).values.single;

      expect(entry.duration, 3.25);
      expect(entry.filename, 'voice-note.m4a');
      expect(entry.size, 42);
      expect(entry.isAudio, isTrue);
    });
  });
}
