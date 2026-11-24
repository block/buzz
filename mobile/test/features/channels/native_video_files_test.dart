import 'dart:async';
import 'dart:io';

import 'package:buzz/features/channels/native_video_files.dart';
import 'package:buzz/shared/relay/media_auth.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:http/http.dart' as http;
import 'package:nostr/nostr.dart' as nostr;

void main() {
  test(
    'video downloads authenticate, retry, cancel and release selected files',
    () async {
      final root = await Directory.systemTemp.createTemp('gallery-video-test-');
      final server = await HttpServer.bind(InternetAddress.loopbackIPv4, 0);
      final client = http.Client();
      final base = 'http://127.0.0.1:${server.port}';
      final auth = MediaGetAuthService(
        baseUrl: base,
        nsec: nostr.Keys.generate().nsec,
      );
      final files = NativeVideoFiles(
        auth: auth,
        client: client,
        index: 0,
        temporaryDirectory: () async => root,
        maxBytes: 8,
      );
      addTearDown(() async {
        await files.dispose();
        client.close();
        await server.close(force: true);
        await root.delete(recursive: true);
      });
      final received = <String>[];
      var fail = true;
      final stalled = Completer<void>();
      server.listen((request) async {
        received.add(request.uri.path);
        expect(request.headers.value('authorization'), startsWith('Nostr '));
        if (request.uri.path.endsWith('/stall.mp4')) {
          request.response.add([1]);
          await request.response.flush();
          stalled.complete();
          return;
        }
        if (request.uri.path.endsWith('/retry.mp4') && fail) {
          request.response.statusCode = 503;
        } else if (request.uri.path.endsWith('/large.mp4')) {
          request.response.add(List.filled(9, 1));
        } else {
          request.response.add([1, 2, 3, 4]);
        }
        await request.response.close();
      });
      expect(await files.load(1, '$base/media/hidden.mp4'), isNull);
      expect(received, isEmpty);
      expect(await files.load(0, '$base/media/retry.mp4'), isNull);
      fail = false;
      final first = await files.load(0, '$base/media/retry.mp4');
      expect(await File(first!).readAsBytes(), [1, 2, 3, 4]);
      await files.select(1);
      expect(await File(first).exists(), isFalse);
      expect(await files.load(1, '$base/media/large.mp4'), isNull);
      expect(await root.list().toList(), isEmpty);
      final pending = files.load(1, '$base/media/stall.mp4');
      await stalled.future.timeout(const Duration(seconds: 5));
      await files.select(2).timeout(const Duration(seconds: 5));
      expect(await pending, isNull);
      final last = await files.load(2, '$base/media/last.mov');
      expect(last, endsWith('.mov'));
      await files.dispose();
      expect(await File(last!).exists(), isFalse);
      expect(await root.list().toList(), isEmpty);
      expect(await files.load(2, '$base/media/after.mp4'), isNull);
    },
  );
}
