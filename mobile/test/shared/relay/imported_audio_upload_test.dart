import 'dart:convert';
import 'dart:io';

import 'package:buzz/shared/relay/media_upload.dart';
import 'package:flutter/services.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:http/http.dart' as http;
import 'package:http/testing.dart';
import 'package:image_picker/image_picker.dart';
import 'package:nostr/nostr.dart' as nostr;

void main() {
  TestWidgetsFlutterBinding.ensureInitialized();

  test(
    'imported recording survives failed preparation, cancel, and retry',
    () async {
      final directory = await Directory.systemTemp.createTemp('audio-import-');
      addTearDown(() => directory.delete(recursive: true));
      final source = File('${directory.path}/회의.M4A');
      await source.writeAsBytes([1, 2, 3]);
      final package = File('${directory.path}/prepared.mp4');
      var failPreparation = true;
      UploadCancellationToken? token;
      final uploads = <List<int>>[];
      final service = MediaUploadService(
        baseUrl: 'https://relay.example',
        pickGalleryImage: () async => null,
        pickGalleryVideo: () async => null,
        nsec: nostr.Keys.generate().nsec,
        pickAttachmentFile: () async => XFile(source.path),
        packageVoiceNoteForUpload: (path) async {
          expect(await File(path).readAsBytes(), [1, 2, 3]);
          if (failPreparation) {
            throw PlatformException(code: 'transcode_failed');
          }
          await package.writeAsBytes([4, 5, 6]);
          token?.cancel();
          return package.path;
        },
        httpClient: MockClient((request) async {
          expect(request.headers['Content-Type'], 'video/mp4');
          uploads.add(request.bodyBytes);
          return http.Response(
            jsonEncode({
              'url': 'https://relay.example/media/recording.mp4',
              'sha256': 'a' * 64,
              'size': request.bodyBytes.length,
              'type': 'video/mp4',
              'uploaded': 1,
              'duration': 120.5,
            }),
            200,
          );
        }),
      );
      addTearDown(service.dispose);
      final retained =
          (await service.pickAttachmentFile())! as RetainedAttachmentFile;
      addTearDown(retained.dispose);
      await source.delete();
      await expectLater(
        service.uploadFile(retained),
        throwsA(isA<PlatformException>()),
      );
      expect(uploads, isEmpty);
      expect(await File(retained.path).exists(), isTrue);

      failPreparation = false;
      token = UploadCancellationToken();
      await expectLater(
        service.uploadFile(retained, cancellationToken: token),
        throwsA(isA<UploadCancelledException>()),
      );
      expect(uploads, isEmpty);
      expect(await package.exists(), isFalse);
      expect(await File(retained.path).exists(), isTrue);

      token = null;
      final descriptor = await service.uploadFile(retained);
      expect(uploads.single, [4, 5, 6]);
      expect(descriptor.filename, 'voice-note-회의.mp4');
      expect(descriptor.duration, 120.5);
      expect(
        descriptor.toMarkdownImage(),
        '[voice-note-회의.mp4](${descriptor.url})',
      );
      expect(await package.exists(), isFalse);
      expect(await File(retained.path).readAsBytes(), [1, 2, 3]);
    },
  );

  test(
    'routes supported audio metadata and bounds input/output before upload',
    () async {
      final directory = await Directory.systemTemp.createTemp('audio-bounds-');
      addTearDown(() => directory.delete(recursive: true));
      final source = File('${directory.path}/source');
      final package = File('${directory.path}/package.mp4');
      var packagingCount = 0;
      var uploads = 0;
      var oversizedOutput = false;
      final service = MediaUploadService(
        baseUrl: 'https://relay.example',
        pickGalleryImage: () async => null,
        pickGalleryVideo: () async => null,
        nsec: nostr.Keys.generate().nsec,
        packageVoiceNoteForUpload: (path) async {
          packagingCount++;
          await package.writeAsBytes([4]);
          if (oversizedOutput) {
            final handle = await package.open(mode: FileMode.append);
            await handle.truncate(100 * 1024 * 1024 + 1);
            await handle.close();
          }
          return package.path;
        },
        httpClient: MockClient((request) async {
          uploads++;
          return http.Response(
            jsonEncode({
              'url': 'https://relay.example/media/file',
              'sha256': 'a' * 64,
              'size': request.bodyBytes.length,
              'type': request.headers['Content-Type'],
              'uploaded': 1,
            }),
            200,
          );
        }),
      );
      addTearDown(service.dispose);
      for (final (name, mime, shouldPackage) in [
        ('memo.m4a', null, true),
        ('memo.AAC', null, true),
        ('memo', 'audio/mp4', true),
        ('memo', 'audio/x-m4a', true),
        ('memo', 'audio/aac', true),
        ('memo.mp3', 'audio/mpeg', false),
        ('video.mp4', 'video/mp4', false),
        ('memo.txt', null, false),
      ]) {
        final file = File('${directory.path}/$name');
        await file.writeAsBytes([1]);
        final before = packagingCount;
        final descriptor = await service.uploadFile(
          XFile(file.path, mimeType: mime),
        );
        expect(packagingCount - before, shouldPackage ? 1 : 0, reason: name);
        expect(descriptor.duration, isNull); // Do not invent a zero duration.
        expect(await file.readAsBytes(), [1]);
      }
      await source.writeAsBytes([]);
      final audio = XFile(source.path, mimeType: 'audio/mp4');
      final before = packagingCount;
      final sent = uploads;
      await expectLater(service.uploadFile(audio), throwsException);
      final handle = await source.open(mode: FileMode.append);
      await handle.truncate(100 * 1024 * 1024 + 1);
      await handle.close();
      await expectLater(service.uploadFile(audio), throwsException);
      expect(packagingCount, before);
      await source.writeAsBytes([1]);
      oversizedOutput = true;
      await expectLater(service.uploadFile(audio), throwsException);
      expect(uploads, sent);
      expect(await package.exists(), isFalse);
      expect(await source.readAsBytes(), [1]);
    },
  );
}
