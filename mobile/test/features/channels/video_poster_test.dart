import 'dart:async';
import 'dart:io';
import 'dart:typed_data';

import 'package:buzz/features/channels/video_poster.dart';
import 'package:buzz/shared/relay/relay.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';
import 'package:http/http.dart' as http;
import 'package:http/testing.dart';
import 'package:nostr/nostr.dart' as nostr;
import 'package:path_provider_platform_interface/path_provider_platform_interface.dart';
import 'package:plugin_platform_interface/plugin_platform_interface.dart';

class _TemporaryPath extends Fake
    with MockPlatformInterfaceMixin
    implements PathProviderPlatform {
  _TemporaryPath(this.path);
  final String path;
  @override
  Future<String?> getTemporaryPath() async => path;
}

class _PosterCache extends MediaDiskCache {
  _PosterCache() : super.open(null);
  final entries = <String, Uint8List>{};
  @override
  Future<MediaDiskCacheHit?> read(String url) async {
    final bytes = entries[url];
    return bytes == null ? null : (bytes: bytes, isStale: false);
  }

  @override
  Future<void> write(
    String url,
    Uint8List bytes, {
    required bool immutable,
  }) async {
    entries[url] = bytes;
  }
}

void main() {
  TestWidgetsFlutterBinding.ensureInitialized();
  test(
    'legacy posters serialize downloads, authenticate, cache and clean files',
    () async {
      final root = await Directory.systemTemp.createTemp('poster-test-');
      final previous = PathProviderPlatform.instance;
      PathProviderPlatform.instance = _TemporaryPath(root.path);
      final release = Completer<void>();
      final extracting = Completer<void>();
      final requests = <http.Request>[];
      final paths = <String>[];
      final client = MockClient((request) async {
        requests.add(request);
        return http.Response.bytes([1, 2, 3], 200);
      });
      final service = MediaUploadService(
        baseUrl: 'https://relay.example',
        nsec: null,
        pickGalleryImage: () async => null,
        pickGalleryVideo: () async => null,
        generateVideoPoster: (path) async {
          paths.add(path);
          expect(await File(path).readAsBytes(), [1, 2, 3]);
          if (paths.length == 1) {
            extracting.complete();
            await release.future;
          }
          return Uint8List.fromList([4, 5, 6]);
        },
      );
      final cache = _PosterCache();
      final container = ProviderContainer(
        overrides: [
          mediaDiskCacheProvider.overrideWithValue(cache),
          mediaGetAuthServiceProvider.overrideWithValue(
            MediaGetAuthService(
              baseUrl: 'https://relay.example',
              nsec: nostr.Keys.generate().nsec,
            ),
          ),
          mediaHttpClientProvider.overrideWithValue(client),
          mediaUploadServiceProvider.overrideWithValue(service),
        ],
      );
      addTearDown(() async {
        container.dispose();
        service.dispose();
        client.close();
        PathProviderPlatform.instance = previous;
        await root.delete(recursive: true);
      });
      final first = videoPosterProvider('https://relay.example/media/one.mp4');
      final second = videoPosterProvider('https://elsewhere.example/two.mp4');
      final sub1 = container.listen(first, (_, _) {});
      final sub2 = container.listen(second, (_, _) {});
      final cancelled = container.listen(
        videoPosterProvider('https://relay.example/media/cancelled.mp4'),
        (_, _) {},
      );
      await extracting.future;
      cancelled.close();
      await container.pump();
      expect(requests, hasLength(1));
      release.complete();
      expect(await container.read(first.future), [4, 5, 6]);
      expect(await container.read(second.future), [4, 5, 6]);
      expect(requests.first.headers['authorization'], startsWith('Nostr '));
      expect(requests.last.headers['authorization'], isNull);
      expect(await root.list().toList(), isEmpty);
      sub1.close();
      sub2.close();
      await container.pump();
      expect(await container.read(first.future), [4, 5, 6]);
      expect(requests, hasLength(2));
    },
  );
}
