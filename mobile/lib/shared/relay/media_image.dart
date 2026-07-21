import 'dart:async';
import 'dart:ui' as ui;

import 'package:flutter/foundation.dart';
import 'package:flutter/material.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';
import 'package:http/http.dart' as http;

import 'media_auth.dart';

/// Shared, keep-alive HTTP client for media fetches. One client per container
/// so TLS connections are reused across images instead of re-handshaking per
/// fetch. Override in tests to stub the network.
final mediaHttpClientProvider = Provider<http.Client>((ref) {
  final client = http.Client();
  ref.onDispose(client.close);
  return client;
});

/// An [ImageProvider] for relay-hosted (and arbitrary remote) media that fixes
/// the fetch-stampede failure modes of `Image.network` + per-build auth
/// headers:
///
/// 1. **Stable cache identity.** Flutter's [NetworkImage] includes the full
///    headers map in its `==`/`hashCode`, so a freshly signed Authorization
///    header on every widget build made the *same URL* a new cache key and
///    bypassed the in-memory [ImageCache] entirely. This provider keys on
///    (url, scale, auth scope) and injects auth headers at *fetch* time, so
///    header refreshes never invalidate cached images.
/// 2. **Failure cooldown.** A URL that fails to load is not retried until a
///    cooldown elapses (honoring `Retry-After` on 429), so error retries on
///    rebuild cannot amplify into a request storm against the rate limiter.
///
/// The auth scope (relay base URL + signing identity) is part of key equality:
/// switching relay or account can never reuse another identity's cached bytes.
class MediaImageProvider extends ImageProvider<MediaImageProvider> {
  final String url;
  final double scale;
  final MediaGetAuthService auth;

  /// Excluded from equality: transport, not identity.
  final http.Client client;

  const MediaImageProvider({
    required this.url,
    required this.auth,
    required this.client,
    this.scale = 1.0,
  });

  static const _defaultCooldown = Duration(seconds: 30);
  static const _maxRetryAfter = Duration(minutes: 5);
  static final Map<String, DateTime> _cooldownUntil = {};

  /// Injectable clock for cooldown tests.
  @visibleForTesting
  static DateTime Function() debugNow = DateTime.now;

  @visibleForTesting
  static void debugResetCooldowns() => _cooldownUntil.clear();

  @override
  Future<MediaImageProvider> obtainKey(ImageConfiguration configuration) {
    return SynchronousFuture<MediaImageProvider>(this);
  }

  @override
  ImageStreamCompleter loadImage(
    MediaImageProvider key,
    ImageDecoderCallback decode,
  ) {
    return MultiFrameImageStreamCompleter(
      codec: _loadAsync(key, decode),
      scale: key.scale,
      debugLabel: url,
    );
  }

  Future<ui.Codec> _loadAsync(
    MediaImageProvider key,
    ImageDecoderCallback decode,
  ) async {
    try {
      final until = _cooldownUntil[url];
      if (until != null) {
        if (debugNow().isBefore(until)) {
          throw MediaImageCooldownException(url: url, until: until);
        }
        _cooldownUntil.remove(url);
      }

      final uri = Uri.parse(url);
      final http.Response response;
      try {
        response = await client.get(uri, headers: auth.headersFor(url));
      } catch (_) {
        _cooldownUntil[url] = debugNow().add(_defaultCooldown);
        rethrow;
      }
      if (response.statusCode != 200) {
        _cooldownUntil[url] = debugNow().add(_cooldownFor(response));
        throw NetworkImageLoadException(
          statusCode: response.statusCode,
          uri: uri,
        );
      }
      final bytes = response.bodyBytes;
      if (bytes.isEmpty) {
        _cooldownUntil[url] = debugNow().add(_defaultCooldown);
        throw NetworkImageLoadException(statusCode: 200, uri: uri);
      }
      final buffer = await ui.ImmutableBuffer.fromUint8List(bytes);
      return decode(buffer);
    } catch (_) {
      // Match NetworkImage: make sure an errored key is not retained in the
      // cache. The cooldown map (not the cache) throttles retries.
      scheduleMicrotask(() {
        PaintingBinding.instance.imageCache.evict(key);
      });
      rethrow;
    }
  }

  Duration _cooldownFor(http.Response response) {
    final retryAfter = int.tryParse(response.headers['retry-after'] ?? '');
    if (retryAfter != null && retryAfter > 0) {
      final requested = Duration(seconds: retryAfter);
      return requested > _maxRetryAfter ? _maxRetryAfter : requested;
    }
    return _defaultCooldown;
  }

  @override
  bool operator ==(Object other) {
    if (other.runtimeType != runtimeType) return false;
    return other is MediaImageProvider &&
        other.url == url &&
        other.scale == scale &&
        other.auth == auth;
  }

  @override
  int get hashCode => Object.hash(url, scale, auth);

  @override
  String toString() =>
      'MediaImageProvider("$url", scale: ${scale.toStringAsFixed(1)})';
}

/// Thrown when a fetch is suppressed because the URL recently failed.
class MediaImageCooldownException implements Exception {
  final String url;
  final DateTime until;

  const MediaImageCooldownException({required this.url, required this.until});

  @override
  String toString() =>
      'MediaImageCooldownException: $url is cooling down until $until';
}

/// Drop-in replacement for the media `Image.network` call sites.
///
/// Renders [url] through [MediaImageProvider] (stable cache key, fetch-time
/// auth, failure cooldown) and bounds the decode size so a handful of large
/// originals cannot thrash the global [ImageCache]:
///
/// - [decodeWidth] set: decode at that logical width (x device pixel ratio).
/// - otherwise, when [boundDecodeToLayout] is true (default): decode at the
///   layout width reported by [LayoutBuilder], when finite.
/// - [boundDecodeToLayout] false and no [decodeWidth]: full-resolution decode
///   (the zoomable full-screen viewer).
class MediaImage extends ConsumerWidget {
  final String url;
  final BoxFit? fit;
  final double? width;
  final double? height;
  final String? semanticLabel;
  final ImageErrorWidgetBuilder? errorBuilder;
  final FilterQuality filterQuality;
  final double? decodeWidth;
  final bool boundDecodeToLayout;

  const MediaImage({
    super.key,
    required this.url,
    this.fit,
    this.width,
    this.height,
    this.semanticLabel,
    this.errorBuilder,
    this.filterQuality = FilterQuality.medium,
    this.decodeWidth,
    this.boundDecodeToLayout = true,
  });

  @override
  Widget build(BuildContext context, WidgetRef ref) {
    final provider = MediaImageProvider(
      url: url,
      auth: ref.watch(mediaGetAuthServiceProvider),
      client: ref.watch(mediaHttpClientProvider),
    );

    if (decodeWidth != null) {
      return _image(provider, _physicalWidth(context, decodeWidth!));
    }
    if (!boundDecodeToLayout) {
      return _image(provider, null);
    }
    return LayoutBuilder(
      builder: (context, constraints) {
        final maxWidth = constraints.maxWidth;
        final cacheWidth = maxWidth.isFinite && maxWidth > 0
            ? _physicalWidth(context, maxWidth)
            : null;
        return _image(provider, cacheWidth);
      },
    );
  }

  int _physicalWidth(BuildContext context, double logicalWidth) {
    return (logicalWidth * MediaQuery.devicePixelRatioOf(context)).ceil();
  }

  Widget _image(MediaImageProvider provider, int? cacheWidth) {
    return Image(
      image: ResizeImage.resizeIfNeeded(cacheWidth, null, provider),
      fit: fit,
      width: width,
      height: height,
      semanticLabel: semanticLabel,
      errorBuilder: errorBuilder,
      filterQuality: filterQuality,
      gaplessPlayback: true,
    );
  }
}
