import 'dart:async';
import 'dart:convert';

import 'package:hooks_riverpod/hooks_riverpod.dart';
import 'package:http/http.dart' as http;

import 'community_icon_cache.dart';
import 'community_icon_artwork.dart';

const _maximumIconBytes = 256 * 1024;
const _lookupTimeout = Duration(seconds: 5);

/// Supplies the unauthenticated client for public NIP-11 documents and icons.
final communityIconHttpClientProvider = Provider<http.Client>((ref) {
  final client = http.Client();
  ref.onDispose(client.close);
  return client;
});

/// Immediately presents saved artwork while a public metadata refresh runs.
final communityIconPresentationProvider = Provider.autoDispose
    .family<String?, String>((ref, relayUrl) {
      final key = _relayInfoUri(relayUrl)?.toString();
      final saved = ref.watch(communityIconCacheProvider)[key];
      final refreshed = ref.watch(communityIconProvider(relayUrl));
      return saved ?? refreshed.value;
    });

/// Refreshes public relay artwork without discarding its last usable copy.
/// Downloaded bytes (including remote images) are persisted, so reopening the
/// picker and offline launches do not depend on the active relay's image cache.
final communityIconProvider = FutureProvider.autoDispose
    .family<String?, String>((ref, relayUrl) async {
      final uri = _relayInfoUri(relayUrl);
      if (uri == null) return null;
      final key = uri.toString();
      final cache = ref.read(communityIconCacheProvider.notifier);
      final saved = ref.read(communityIconCacheProvider)[key];
      final client = ref.read(communityIconHttpClientProvider);
      var disposed = false;
      final cancelled = Completer<void>();
      ref.onDispose(() {
        disposed = true;
        cancelled.complete();
      });
      try {
        final response = await _download(
          client,
          uri,
          cancelled.future,
          metadata: true,
        );
        if (response == null || disposed) return saved;
        final document = jsonDecode(utf8.decode(response.bytes));
        if (document is! Map<String, dynamic>) return saved;
        final value = document['icon'];
        if (value == null || (value is String && value.trim().isEmpty)) {
          await cache.remember(key, null);
          return null;
        }
        if (value is! String) return saved;
        final icon = value.trim();
        String? artwork;
        if (icon.startsWith('data:image/')) {
          final data = UriData.parse(icon);
          if (data.contentAsBytes().length <= _maximumIconBytes) artwork = icon;
        } else {
          final imageUri = Uri.tryParse(icon);
          if (imageUri != null &&
              (imageUri.scheme == 'https' || imageUri.scheme == 'http')) {
            final image = await _download(client, imageUri, cancelled.future);
            if (image != null && image.mimeType.startsWith('image/')) {
              artwork = Uri.dataFromBytes(
                image.bytes,
                mimeType: image.mimeType,
              ).toString();
            }
          }
        }
        if (disposed) return saved;
        // Never expose rejected remote URLs to the avatar renderer: doing so
        // would bypass the bounded download above.
        if (artwork == null) return saved;
        final prepared = await prepareCommunityIconArtwork(artwork);
        if (prepared == null || disposed) return saved;
        await cache.remember(key, prepared);
        return prepared;
      } catch (_) {
        return saved;
      }
    });

Future<({List<int> bytes, String mimeType})?> _download(
  http.Client client,
  Uri uri,
  Future<void> cancelled, {
  bool metadata = false,
}) async {
  final abort = Completer<void>();
  void cancel() {
    if (!abort.isCompleted) abort.complete();
  }

  unawaited(cancelled.then((_) => cancel()));
  final request = http.AbortableRequest('GET', uri, abortTrigger: abort.future);
  if (metadata) request.headers['Accept'] = 'application/nostr+json';
  StreamIterator<List<int>>? iterator;
  try {
    final response = await client.send(request).timeout(_lookupTimeout);
    iterator = StreamIterator(response.stream);
    final limit = metadata ? _maximumIconBytes * 2 : _maximumIconBytes;
    if (response.statusCode < 200 ||
        response.statusCode >= 300 ||
        (response.contentLength ?? 0) > limit) {
      return null;
    }
    final deadline = DateTime.now().add(_lookupTimeout);
    final bytes = <int>[];
    while (await iterator.moveNext().timeout(
      deadline.difference(DateTime.now()),
    )) {
      if (bytes.length + iterator.current.length > limit) return null;
      bytes.addAll(iterator.current);
    }
    return (
      bytes: bytes,
      mimeType: (response.headers['content-type'] ?? '')
          .split(';')
          .first
          .trim()
          .toLowerCase(),
    );
  } finally {
    // Future.timeout does not stop the underlying socket, including while
    // waiting for response headers. Abort before releasing the stream.
    cancel();
    await iterator?.cancel();
  }
}

Uri? _relayInfoUri(String relayUrl) {
  try {
    final uri = Uri.parse(relayUrl.trim());
    final scheme = switch (uri.scheme) {
      'wss' => 'https',
      'ws' => 'http',
      'https' || 'http' => uri.scheme,
      _ => null,
    };
    return scheme == null || uri.host.isEmpty
        ? null
        : uri
              .replace(scheme: scheme, path: uri.path.isEmpty ? '/' : uri.path)
              .removeFragment();
  } on FormatException {
    return null;
  }
}
