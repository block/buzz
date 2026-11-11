import 'dart:async';
import 'dart:typed_data';

import 'package:hooks_riverpod/hooks_riverpod.dart';

import '../../shared/relay/relay.dart';
import 'native_video_files.dart';

// Serialize legacy-poster work so a timeline never launches many video downloads
// or decoders at once. This queue belongs to the current provider container.
final _posterQueueProvider = Provider((ref) => _PosterQueue());

class _PosterQueue {
  Future<void> tail = Future.value();
  Future<T> run<T>(Future<T> Function() work) {
    final result = tail.then((_) => work());
    tail = result.then<void>((_) {}, onError: (Object _, StackTrace _) {});
    return result;
  }
}

/// Generates a still for older video messages that have no poster metadata.
/// Downloads are bounded/cancellable and temporary video files are removed.
/// Stills use the account/community image cache; no playback controller is created.
final videoPosterProvider = FutureProvider.autoDispose
    .family<Uint8List?, String>((ref, url) async {
      final auth = ref.watch(mediaGetAuthServiceProvider);
      final client = ref.watch(mediaHttpClientProvider);
      final service = ref.watch(mediaUploadServiceProvider);
      final queue = ref.watch(_posterQueueProvider);
      final cache = ref.watch(mediaDiskCacheProvider);
      final cacheKey = 'video-poster:v1:$url';
      final files = NativeVideoFiles(auth: auth, client: client, index: 0);
      var disposed = false;
      ref.onDispose(() {
        disposed = true;
        unawaited(files.dispose());
      });
      final cached = await cache?.read(cacheKey);
      if (disposed) return null;
      if (cached != null && !cached.isStale) return cached.bytes;
      return queue.run(() async {
        if (disposed) return null;
        try {
          final path = await files.load(0, url);
          if (path == null || disposed) return null;
          final bytes = await service.prepareVideoPoster(path);
          if (!disposed && bytes != null && bytes.isNotEmpty) {
            await cache?.write(
              cacheKey,
              bytes,
              immutable: auth.isRelayMediaUrl(url),
            );
          }
          return bytes;
        } finally {
          await files.dispose();
        }
      });
    });
