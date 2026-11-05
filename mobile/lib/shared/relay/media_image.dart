import 'dart:async';
import 'dart:ui' as ui;

import 'package:flutter/foundation.dart';
import 'package:flutter/material.dart';
import 'package:flutter_hooks/flutter_hooks.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';
import 'package:http/http.dart' as http;

import '../hosted/hosted_relay_auth.dart';
import '../utils/bounded_fetch.dart';
import 'media_auth.dart';
import 'media_disk_cache.dart';

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
///    (url, scale, auth scope) and injects auth headers
///    at *fetch* time, so header refreshes never invalidate cached images.
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

  /// Where loaded bytes persist across launches, or null to keep them in
  /// memory only. Excluded from equality: storage, not identity.
  final MediaDiskCache? diskCache;

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
    this.diskCache,
    this.onBytesLoaded,
    this.scale = 1.0,
    this.stallTimeout = defaultStallTimeout,
    this.maxFetchDuration = defaultMaxFetchDuration,
  });

  /// Maximum encoded image body retained before decoding (20 MiB).
  static const maxResponseBytes = 20 * 1024 * 1024;

  static const _defaultCooldown = Duration(seconds: 30);
  static const _maxRetryAfter = Duration(minutes: 5);

  static final Map<String, DateTime> _cooldownUntil = {};
  static final Map<(String, MediaGetAuthService), _SharedFetch> _inFlight = {};

  /// URLs with a background refresh of stale disk bytes under way.
  static final Set<String> _refreshing = {};

  /// Downloaded bytes not yet saved to disk, by URL: they are saved once one
  /// load decodes a frame from them, so a corrupt body is never persisted.
  static final Map<
    String,
    ({MediaDiskCache cache, Uint8List bytes, bool immutable})
  >
  _unsaved = {};

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
    // only once a consumer evicts the entry and stops listening (see
    // [abandonPendingImage]), as [MediaImage] does when it stops showing an
    // image before its first frame. Abort the download then.
    return _MediaImageStreamCompleter(
      codec: _loadAsync(key, decode, abandoned.future),
      scale: key.scale,
      debugLabel: url,
      onFirstFrame: _saveDecoded,
      onError: () => _unsaved.remove(url),
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
        _loadBytes,
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

  /// [url]'s bytes from [diskCache] when saved there, else downloaded (and
  /// saved once they decode). Stale saved bytes are shown while a background download
  /// replaces them.
  Future<Uint8List> _loadBytes(Future<void> abort) async {
    final cache = diskCache;
    if (cache == null) return _fetchBytes(abort);
    final hit = await cache.read(url);
    if (hit != null) {
      if (hit.isStale) unawaited(_refresh(cache));
      return hit.bytes;
    }
    final bytes = await _fetchBytes(abort);
    _unsaved[url] = (
      cache: cache,
      bytes: bytes,
      immutable: _isContentAddressed,
    );
    return bytes;
  }

  void _saveDecoded() {
    if (_unsaved.remove(url) case final save?) {
      unawaited(save.cache.write(url, save.bytes, immutable: save.immutable));
    }
  }

  Future<void> _refresh(MediaDiskCache cache) async {
    if (!_refreshing.add(url)) return;
    try {
      // Never aborted: nobody is waiting on it.
      final bytes = await _fetchBytes(Completer<void>().future);
      // Nothing displays these bytes yet, so check they decode before they
      // replace the saved copy.
      final codec = await ui.instantiateImageCodec(bytes);
      try {
        (await codec.getNextFrame()).image.dispose();
      } finally {
        codec.dispose();
      }
      await cache.write(url, bytes, immutable: _isContentAddressed);
    } on Object catch (error) {
      // The stale bytes stay; the next load after the cooldown retries.
      debugPrint('[MediaImageProvider] refresh of $url failed: $error');
    } finally {
      _refreshing.remove(url);
    }
  }

  /// A relay blob or its thumbnail: named by content hash, so its bytes
  /// never change.
  bool get _isContentAddressed {
    if (!auth.isRelayMediaUrl(url)) return false;
    final uri = Uri.parse(url);
    return !uri.hasQuery && _relayContentPath.hasMatch(uri.path);
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
    final BoundedResponse response;
    try {
      response = await boundedGet(
        client,
        uri,
        maxBytes: maxResponseBytes,
        headers: auth.headersFor(url),
        stallTimeout: stallTimeout,
        maxFetchDuration: maxFetchDuration,
        cancel: abort,
      );
    } catch (error) {
      final wasAbandoned = error is FetchCancelledException;
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
    final bytes = response.bytes;
    if (bytes.isEmpty) {
      _cooldownUntil[url] = debugNow().add(_defaultCooldown);
      throw NetworkImageLoadException(statusCode: 200, uri: uri);
    }
    return bytes;
  }

  Duration _cooldownFor(BoundedResponse response) {
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
    required this.onFirstFrame,
    required this.onError,
    super.debugLabel,
  });

  /// Called once, when the first frame has decoded. A codec can be created
  /// from bytes whose frames then fail to decode.
  final VoidCallback onFirstFrame;
  final VoidCallback onError;
  var _hasFrame = false;

  final _listeners = <ImageStreamListener>[];

  @override
  void setImage(ImageInfo image) {
    if (!_hasFrame) {
      _hasFrame = true;
      onFirstFrame();
    }
    super.setImage(image);
  }

  @override
  void reportError({
    DiagnosticsNode? context,
    required Object exception,
    StackTrace? stack,
    InformationCollector? informationCollector,
    bool silent = false,
  }) {
    onError();
    super.reportError(
      context: context,
      exception: exception,
      stack: stack,
      informationCollector: informationCollector,
      silent: silent,
    );
  }

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

/// One download shared by every load of the same URL and auth scope. It is aborted once every load waiting for
/// it has been abandoned, and a load that arrives after that starts a new
/// download.
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

final _relayContentPath = RegExp(
  r'^/media/[0-9a-f]{64}(?:\.thumb\.jpg|\.[a-z0-9]{1,8})?$',
);

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
/// relay's generated thumbnail instead of the original upload. It never falls
/// back to the original: a missing thumbnail shows [errorBuilder], so a small
/// image cannot download and decode a multi-megabyte upload.
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
         'a thumbnail failure needs an error widget',
       );

  @override
  Widget build(BuildContext context, WidgetRef ref) {
    final auth = ref.watch(mediaGetAuthServiceProvider);
    final client = ref.watch(mediaHttpClientProvider);
    final diskCache = ref.watch(mediaDiskCacheProvider);
    MediaImageProvider providerFor(String source) => MediaImageProvider(
      url: source,
      auth: auth,
      client: client,
      diskCache: diskCache,
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
        final thumbnail =
            preferThumbnail && _fitsThumbnail(context, constraints)
            ? relayThumbnailUrl(url, auth)
            : null;
        return _image(providerFor(thumbnail ?? url), cacheWidth);
      },
    );
  }

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

  Widget _image(MediaImageProvider provider, int? cacheWidth) {
    final image = ResizeImage.resizeIfNeeded(cacheWidth, null, provider);
    return _AbandonWhenGone(
      image: image,
      child: Image(
        image: image,
        fit: fit,
        width: width,
        height: height,
        semanticLabel: semanticLabel,
        errorBuilder: errorBuilder,
        frameBuilder: frameBuilder,
        filterQuality: filterQuality,
        gaplessPlayback: true,
      ),
    );
  }
}

/// Abandons [image]'s download if [child] stops showing it before it loads,
/// for example when a chat image scrolls away. Otherwise the image cache's
/// own listener would keep the download, and its share of a slow link, going
/// until the fetch cap. Another widget still showing the image keeps it.
class _AbandonWhenGone extends HookWidget {
  const _AbandonWhenGone({required this.image, required this.child});

  final ImageProvider<Object> image;
  final Widget child;

  @override
  Widget build(BuildContext context) {
    final config = createLocalImageConfiguration(context);
    // Resolve only once the child is gone, and only a still-pending load:
    // resolving earlier would start the download ahead of the child's
    // [ScrollAwareImageProvider], defeating its deferral during a fast fling.
    // The microtask runs after this frame, so the child has stopped
    // listening and a widget showing the same image elsewhere has started.
    useEffect(
      () =>
          () => scheduleMicrotask(() {
            final cache = PaintingBinding.instance.imageCache;
            unawaited(
              image.obtainKey(config).then((key) {
                if (!cache.statusForKey(key).pending) return;
                abandonPendingImage(image.resolve(config), image, config);
              }),
            );
          }),
      [image],
    );
    return child;
  }
}
