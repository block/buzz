import 'dart:async';
import 'dart:typed_data';
import 'dart:ui' as ui;

import 'package:flutter/foundation.dart';
import 'package:flutter/material.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';
import 'package:http/http.dart' as http;

import '../hosted/hosted_relay_auth.dart';
import 'media_auth.dart';

/// Shared, keep-alive HTTP client for media fetches. One client per container
/// so TLS connections are reused across images instead of re-handshaking per
/// fetch. Override in tests to stub the network.
final mediaHttpClientProvider = Provider<http.Client>((ref) {
  final client = ref.watch(hostedHttpClientFactoryProvider)();
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

  /// Optional observer for bytes already fetched by the foreground image path.
  /// Excluded from equality because it is a side effect, not cache identity.
  final ValueChanged<Uint8List>? onBytesLoaded;

  /// Excluded from equality: transport, not identity.
  final http.Client client;

  /// Longest wait for response headers or the next body chunk. A slow link
  /// that keeps delivering bytes finishes; one that stops is aborted without
  /// closing the shared client.
  final Duration stallTimeout;

  /// Longest a fetch may take even while bytes keep arriving, so a server
  /// that trickles bytes cannot hold a connection indefinitely.
  final Duration maxFetchDuration;

  const MediaImageProvider({
    required this.url,
    required this.auth,
    required this.client,
    this.onBytesLoaded,
    this.scale = 1.0,
    this.stallTimeout = const Duration(seconds: 20),
    this.maxFetchDuration = const Duration(minutes: 5),
  });

  /// Maximum encoded image body retained before decoding (20 MiB).
  static const maxResponseBytes = 20 * 1024 * 1024;

  static const _defaultCooldown = Duration(seconds: 30);
  static const _maxRetryAfter = Duration(minutes: 5);
  static final Map<String, DateTime> _cooldownUntil = {};
  static final Map<(String, MediaGetAuthService), _SharedFetch> _inFlight = {};

  /// Injectable clock for cooldown tests.
  @visibleForTesting
  static DateTime Function() debugNow = DateTime.now;

  @visibleForTesting
  static void debugResetCooldowns() => _cooldownUntil.clear();

  @visibleForTesting
  static Iterable<String> get debugCooldownUrls => _cooldownUntil.keys;

  @override
  Future<MediaImageProvider> obtainKey(ImageConfiguration configuration) {
    return SynchronousFuture<MediaImageProvider>(this);
  }

  @override
  ImageStreamCompleter loadImage(
    MediaImageProvider key,
    ImageDecoderCallback decode,
  ) {
    final abandoned = Completer<void>();
    // The image cache listens to a pending load, so this fires mid-download
    // only once a consumer evicts the entry and stops listening, as the iOS
    // carousel does for a preview scrolled past. Abort the download then.
    return _MediaImageStreamCompleter(
      codec: _loadAsync(key, decode, abandoned.future),
      scale: key.scale,
      debugLabel: url,
    )..addOnLastListenerRemovedCallback(() {
      if (!abandoned.isCompleted) abandoned.complete();
    });
  }

  Future<ui.Codec> _loadAsync(
    MediaImageProvider key,
    ImageDecoderCallback decode,
    Future<void> abandoned,
  ) async {
    var wasAbandoned = false;
    try {
      // Each decode size is its own cache entry (see [ResizeImage]), so one
      // URL shown at two sizes resolves twice. Share the download instead of
      // fetching the same bytes once per size.
      final fetchKey = (url, auth);
      final fetch = _inFlight[fetchKey] ??= _SharedFetch(
        _fetchBytes,
        onFinished: (fetch) {
          if (_inFlight[fetchKey] == fetch) _inFlight.remove(fetchKey);
        },
      );
      fetch.join();
      var left = false;
      void leave() {
        if (left) return;
        left = true;
        fetch.leave();
      }

      unawaited(
        abandoned.then((_) {
          wasAbandoned = true;
          leave();
        }),
      );
      final Uint8List bytes;
      try {
        bytes = await fetch.bytes;
      } finally {
        leave();
      }
      // Another size kept the download going, but this load's consumer left.
      if (wasAbandoned) throw StateError('Image load abandoned');
      onBytesLoaded?.call(bytes);
      final buffer = await ui.ImmutableBuffer.fromUint8List(bytes);
      return await decode(buffer);
    } catch (_) {
      // An abandoned load has no listener left to report to, and an error
      // would surface as an unhandled image error. The consumer already
      // evicted it; leave the codec unresolved.
      if (wasAbandoned) return Completer<ui.Codec>().future;
      // Match NetworkImage: make sure an errored key is not retained in the
      // cache. The cooldown map (not the cache) throttles retries.
      scheduleMicrotask(() {
        PaintingBinding.instance.imageCache.evict(key);
      });
      rethrow;
    }
  }

  /// Downloads [url]. Completing [abort] means every load waiting for this
  /// download was abandoned, so the URL is not put on cooldown.
  Future<Uint8List> _fetchBytes(Future<void> abort) async {
    final until = _cooldownUntil[url];
    if (until != null) {
      if (debugNow().isBefore(until)) {
        throw MediaImageCooldownException(url: url, until: until);
      }
      _cooldownUntil.remove(url);
    }

    final uri = Uri.parse(url);
    final http.Response response;
    var wasAbandoned = false;
    try {
      final stop = Completer<void>();
      Timer? stall;
      void restartStallTimer() {
        stall?.cancel();
        stall = Timer(stallTimeout, () {
          if (!stop.isCompleted) stop.complete();
        });
      }

      restartStallTimer();
      final deadline = Timer(maxFetchDuration, () {
        if (!stop.isCompleted) stop.complete();
      });
      unawaited(
        abort.then((_) {
          if (stop.isCompleted) return;
          wasAbandoned = true;
          stop.complete();
        }),
      );
      StreamIterator<List<int>>? body;
      try {
        final request = http.AbortableRequest(
          'GET',
          uri,
          abortTrigger: stop.future,
        )..headers.addAll(auth.headersFor(url));
        final streamed = await client.send(request);
        restartStallTimer();
        final iterator = body = StreamIterator(streamed.stream);
        final bytes = BytesBuilder(copy: false);
        // An error body is never shown, and reading a slow one could hold
        // this load for the whole fetch cap before the fallback or
        // Retry-After applies. Leave it unread; finally cancels the stream.
        while (streamed.statusCode == 200 && await iterator.moveNext()) {
          restartStallTimer();
          final chunk = iterator.current;
          if (chunk.length > maxResponseBytes - bytes.length) {
            throw const FormatException('Image response too large');
          }
          bytes.add(chunk);
        }
        response = http.Response.bytes(
          bytes.takeBytes(),
          streamed.statusCode,
          headers: streamed.headers,
        );
      } finally {
        stall?.cancel();
        deadline.cancel();
        if (!stop.isCompleted) stop.complete();
        unawaited(body?.cancel());
      }
    } catch (_) {
      // Nobody wanted the image any more; the URL itself did not fail.
      if (!wasAbandoned) {
        _cooldownUntil[url] = debugNow().add(_defaultCooldown);
      }
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
    return bytes;
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

/// Stops waiting for [provider]'s image before its first frame arrives.
///
/// Call after removing your own listener from [stream]. Until the first frame
/// the image cache holds its own listener on the load, so removing a caller's
/// listener alone leaves the download running. When no other consumer is
/// listening, evicting the pending entry drops that last listener and a
/// [MediaImageProvider] load aborts its download. When another consumer still
/// listens, the entry stays so the finished image is still cached.
void abandonPendingImage(
  ImageStream stream,
  ImageProvider<Object> provider,
  ImageConfiguration config,
) {
  final completer = stream.completer;
  if (completer is _MediaImageStreamCompleter &&
      completer._listeners.length != 1) {
    // Someone besides the cache still wants it, or it already finished.
    return;
  }
  unawaited(
    provider
        .obtainKey(config)
        .then(
          (key) => PaintingBinding.instance.imageCache.evict(
            key,
            includeLive: false,
          ),
        ),
  );
}

/// Tracks its listeners so [abandonPendingImage] can tell whether anyone
/// besides the image cache is still waiting for the load.
class _MediaImageStreamCompleter extends MultiFrameImageStreamCompleter {
  _MediaImageStreamCompleter({
    required super.codec,
    required super.scale,
    super.debugLabel,
  });

  final _listeners = <ImageStreamListener>[];

  @override
  void addListener(ImageStreamListener listener) {
    _listeners.add(listener);
    super.addListener(listener);
  }

  @override
  void removeListener(ImageStreamListener listener) {
    _listeners.remove(listener);
    super.removeListener(listener);
  }
}

/// One download shared by every load of the same URL and auth scope. It is
/// aborted once every load waiting for it has been abandoned, and a load that
/// arrives after that starts a new download.
class _SharedFetch {
  _SharedFetch(
    Future<Uint8List> Function(Future<void> abort) fetch, {
    required this.onFinished,
  }) {
    bytes = fetch(_abort.future).whenComplete(() {
      _done = true;
      onFinished(this);
    });
    // Waiting loads handle errors; an abandoned download has none left.
    bytes.ignore();
  }

  final void Function(_SharedFetch) onFinished;
  final _abort = Completer<void>();
  late final Future<Uint8List> bytes;
  var _waiting = 0;
  var _done = false;

  void join() => _waiting++;

  /// Called once per joined load, when it is abandoned or has its result.
  void leave() {
    _waiting--;
    if (_waiting == 0 && !_done && !_abort.isCompleted) {
      _abort.complete();
      onFinished(this);
    }
  }
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

final _relayBlobPath = RegExp(r'^/media/([0-9a-f]{64})(?:\.[a-z0-9]{1,8})?$');

/// The relay's generated thumbnail for a relay-hosted image, or null when
/// [url] is not one. Every image uploaded to a relay gets a JPEG thumbnail at
/// `/media/<sha256>.thumb.jpg`, at most 320px on its longer side. This
/// matches desktop's small avatars (buzz-app `mediaUrl`), including the cost:
/// the thumbnail is a still JPEG, so transparent areas render black and
/// animation is lost.
String? relayThumbnailUrl(String url, MediaGetAuthService auth) {
  if (!auth.isRelayMediaUrl(url)) return null;
  final uri = Uri.parse(url);
  if (uri.hasQuery || uri.hasFragment) return null;
  final match = _relayBlobPath.firstMatch(uri.path);
  if (match == null) return null;
  return uri.replace(path: '/media/${match[1]}.thumb.jpg').toString();
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
///
/// With [preferThumbnail], a relay image laid out small enough loads the
/// relay's generated thumbnail instead of the original upload, falling back
/// to the original when the relay has no thumbnail for it.
class MediaImage extends ConsumerWidget {
  final String url;
  final BoxFit? fit;
  final double? width;
  final double? height;
  final String? semanticLabel;
  final ImageErrorWidgetBuilder? errorBuilder;
  final ImageFrameBuilder? frameBuilder;
  final FilterQuality filterQuality;
  final double? decodeWidth;
  final bool boundDecodeToLayout;
  final ValueChanged<Uint8List>? onBytesLoaded;

  /// Load the relay's thumbnail when the laid-out image is at most
  /// [thumbnailMaxPhysicalSize] pixels on each side. Avatars set this: an
  /// original can be megabytes while it renders a few dozen points wide.
  final bool preferThumbnail;

  /// Relay thumbnails are at most 320px on their longer side, so a 4:3 one is
  /// 240px on its shorter side. At or below this size, a cover-cropped
  /// thumbnail of a 4:3 or squarer image is not upscaled.
  static const thumbnailMaxPhysicalSize = 240.0;

  const MediaImage({
    super.key,
    required this.url,
    this.fit,
    this.width,
    this.height,
    this.semanticLabel,
    this.errorBuilder,
    this.frameBuilder,
    this.filterQuality = FilterQuality.medium,
    this.decodeWidth,
    this.boundDecodeToLayout = true,
    this.onBytesLoaded,
    this.preferThumbnail = false,
  }) : assert(
         !preferThumbnail || (decodeWidth == null && boundDecodeToLayout),
         'preferThumbnail needs the layout size to choose a source',
       ),
       assert(
         !preferThumbnail || onBytesLoaded == null,
         'thumbnail bytes are not the original upload',
       ),
       assert(
         !preferThumbnail || errorBuilder != null,
         'a thumbnail failure other than 404 needs an error widget',
       );

  @override
  Widget build(BuildContext context, WidgetRef ref) {
    final auth = ref.watch(mediaGetAuthServiceProvider);
    final client = ref.watch(mediaHttpClientProvider);
    MediaImageProvider providerFor(String source) => MediaImageProvider(
      url: source,
      auth: auth,
      client: client,
      onBytesLoaded: onBytesLoaded,
    );

    if (decodeWidth != null) {
      return _image(providerFor(url), _physicalWidth(context, decodeWidth!));
    }
    if (!boundDecodeToLayout) {
      return _image(providerFor(url), null);
    }
    return LayoutBuilder(
      builder: (context, constraints) {
        final maxWidth = constraints.maxWidth;
        final cacheWidth = maxWidth.isFinite && maxWidth > 0
            ? _physicalWidth(context, maxWidth)
            : null;
        final candidate =
            preferThumbnail && _fitsThumbnail(context, constraints)
            ? relayThumbnailUrl(url, auth)
            : null;
        final thumbnail = _missingThumbnails.contains(candidate)
            ? null
            : candidate;
        if (thumbnail == null) return _image(providerFor(url), cacheWidth);
        return _image(
          providerFor(thumbnail),
          cacheWidth,
          errorBuilder: (context, error, stackTrace) {
            if (error is! NetworkImageLoadException ||
                error.statusCode != 404) {
              return errorBuilder!(context, error, stackTrace);
            }
            // Older or externally populated media may have no generated
            // thumbnail. Remember that, so later builds go straight to the
            // original instead of hitting the thumbnail's failure cooldown.
            _rememberMissingThumbnail(thumbnail);
            return _image(providerFor(url), cacheWidth);
          },
        );
      },
    );
  }

  /// Thumbnails the relay returned 404 for, oldest first. Bounded so a
  /// community full of pre-thumbnail media cannot grow it without limit; an
  /// evicted entry costs one more 404 before the original loads again.
  static final Set<String> _missingThumbnails = <String>{};
  static const _maxMissingThumbnails = 256;

  static void _rememberMissingThumbnail(String url) {
    // This set now stands in for the 404's failure cooldown. Drop that, or
    // nothing would fetch the URL again to clear it, and an entry evicted
    // here during its cooldown would show the fallback, not the original.
    MediaImageProvider._cooldownUntil.remove(url);
    _missingThumbnails
      ..remove(url)
      ..add(url);
    if (_missingThumbnails.length > _maxMissingThumbnails) {
      _missingThumbnails.remove(_missingThumbnails.first);
    }
  }

  @visibleForTesting
  static int get debugMissingThumbnailCount => _missingThumbnails.length;

  @visibleForTesting
  static void debugResetMissingThumbnails() => _missingThumbnails.clear();

  bool _fitsThumbnail(BuildContext context, BoxConstraints constraints) {
    if (!constraints.hasBoundedWidth || !constraints.hasBoundedHeight) {
      return false;
    }
    final longest =
        constraints.biggest.longestSide *
        MediaQuery.devicePixelRatioOf(context);
    return longest <= thumbnailMaxPhysicalSize;
  }

  int _physicalWidth(BuildContext context, double logicalWidth) {
    return (logicalWidth * MediaQuery.devicePixelRatioOf(context)).ceil();
  }

  Widget _image(
    MediaImageProvider provider,
    int? cacheWidth, {
    ImageErrorWidgetBuilder? errorBuilder,
  }) {
    return Image(
      image: ResizeImage.resizeIfNeeded(cacheWidth, null, provider),
      fit: fit,
      width: width,
      height: height,
      semanticLabel: semanticLabel,
      errorBuilder: errorBuilder ?? this.errorBuilder,
      frameBuilder: frameBuilder,
      filterQuality: filterQuality,
      gaplessPlayback: true,
    );
  }
}
