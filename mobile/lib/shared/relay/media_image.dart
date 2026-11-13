import 'dart:async';
import 'dart:collection';
import 'dart:ui' as ui;

import 'package:flutter/foundation.dart';
import 'package:flutter/material.dart';
import 'package:flutter_hooks/flutter_hooks.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';
import 'package:http/http.dart' as http;
import 'package:buzz/shared/http/app_http_client.dart';

import '../hosted/hosted_relay_auth.dart';
import '../utils/bounded_fetch.dart';
import 'media_auth.dart';
import 'media_disk_cache.dart';

/// Shared, keep-alive HTTP client for media fetches. One client per container
/// so TLS connections are reused across images instead of re-handshaking per
/// fetch. Override in tests to stub the network.
final mediaHttpClientProvider = Provider<http.Client>((ref) {
  final client = ref.watch(hostedHttpClientFactoryProvider)(
    traffic: HttpTraffic.media,
  );
  ref.onDispose(client.close);
  return client;
});

/// The provider [MediaImage] shows [url] with at full size (no decode
/// bound), for warming it with `precacheImage`.
MediaImageProvider mediaImageProviderFor(WidgetRef ref, String url) =>
    MediaImageProvider(
      url: url,
      auth: ref.read(mediaGetAuthServiceProvider),
      client: ref.read(mediaHttpClientProvider),
      diskCache: ref.read(mediaDiskCacheProvider),
    );

/// The providers to try, best first, for [url] drawn at most
/// [MediaImage.thumbnailMaxPhysicalSize] physical pixels outside a
/// [MediaImage], such as a native bar's avatar bitmap. This is the source
/// [MediaImage.preferThumbnail] picks: a relay image gets only its generated
/// thumbnails, minus those the relay already returned 404 for, so it can get
/// none; any other URL gets itself. Try the next provider only after a 404.
List<MediaImageProvider> smallMediaImageProviders({
  required String url,
  required MediaGetAuthService auth,
  required http.Client client,
  MediaDiskCache? diskCache,
}) {
  final thumbnails = relayThumbnailUrls(url, auth);
  return [
    if (thumbnails.isEmpty)
      MediaImageProvider(
        url: url,
        auth: auth,
        client: client,
        diskCache: diskCache,
      )
    else
      for (final thumbnail in thumbnails)
        if (!MediaImage._missingThumbnails.contains(thumbnail))
          MediaImageProvider(
            url: thumbnail,
            auth: auth,
            client: client,
            diskCache: diskCache,
            isThumbnailFallback: true,
          ),
  ];
}

/// An [ImageProvider] for relay-hosted (and arbitrary remote) media that fixes
/// the fetch-stampede failure modes of `Image.network` + per-build auth
/// headers:
///
/// 1. **Stable cache identity.** Flutter's [NetworkImage] includes the full
///    headers map in its `==`/`hashCode`, so a freshly signed Authorization
///    header on every widget build made the *same URL* a new cache key and
///    bypassed the in-memory [ImageCache] entirely. This provider keys on
///    (url, scale, auth scope, isThumbnailFallback) and injects auth headers
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

  /// Set when [url] is a thumbnail [MediaImage] chose in place of an
  /// original: its 404 then records a missing thumbnail, so later builds try
  /// the next thumbnail, rather than a failure cooldown. Any other 404, including a
  /// profile URL that is itself a thumbnail, keeps the cooldown. Part of
  /// equality, the shared download's key and the cooldown's key, so a
  /// fallback and a direct load of the same URL each keep their own failure
  /// handling.
  final bool isThumbnailFallback;

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
    this.isThumbnailFallback = false,
    this.scale = 1.0,
    this.stallTimeout = defaultStallTimeout,
    this.maxFetchDuration = defaultMaxFetchDuration,
  });

  /// Maximum encoded image body retained before decoding (20 MiB).
  static const maxResponseBytes = 20 * 1024 * 1024;

  static const _defaultCooldown = Duration(seconds: 30);
  static const _maxRetryAfter = Duration(minutes: 5);

  /// Keyed by (url, isThumbnailFallback), like [_inFlight] minus auth: a
  /// direct load's failure must not stop a fallback to the same URL from
  /// recording a missing thumbnail.
  static final Map<(String, bool), DateTime> _cooldownUntil = {};
  static final Map<(String, MediaGetAuthService, bool), _SharedFetch>
  _inFlight = {};

  /// Background refreshes of stale disk bytes running or queued, per cache
  /// and URL.
  static final Set<(MediaDiskCache, String)> _refreshing = {};
  static final _refreshQueue = Queue<Future<void> Function()>();
  static var _runningRefreshes = 0;

  /// Most background refreshes at once; the rest wait in a queue.
  static const _maxRefreshes = 2;

  /// Most refreshes waiting. A stale image dropped past this keeps its
  /// saved bytes and refreshes the next time it loads from disk.
  static const _maxQueuedRefreshes = 64;

  /// Injectable clock for cooldown tests.
  @visibleForTesting
  static DateTime Function() debugNow = DateTime.now;

  @visibleForTesting
  static void debugResetCooldowns() => _cooldownUntil.clear();

  @visibleForTesting
  static Iterable<String> get debugCooldownUrls =>
      _cooldownUntil.keys.map((key) => key.$1);

  (String, bool) get _cooldownKey => (url, isThumbnailFallback);

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
    _SharedFetch? fetched;
    // The image cache listens to a pending load, so this fires mid-download
    // only once a consumer evicts the entry and stops listening (see
    // [abandonPendingImage]), as [MediaImage] does when it stops showing an
    // image before its first frame. Abort the download then.
    return _MediaImageStreamCompleter(
      codec: _loadAsync(
        key,
        decode,
        abandoned.future,
        onFetch: (fetch) => fetched = fetch,
      ),
      scale: key.scale,
      debugLabel: url,
      // Save only bytes this load decoded a frame from (a codec can open
      // from bytes whose frames then fail to decode), and drop saved bytes
      // that no longer decode. Then drop the fetch: the cached completer
      // must not keep the encoded bytes alive.
      onSettled: (decoded) {
        fetched?.settle(decoded: decoded);
        fetched = null;
      },
    )..addOnLastListenerRemovedCallback(() {
      if (!abandoned.isCompleted) abandoned.complete();
    });
  }

  Future<ui.Codec> _loadAsync(
    MediaImageProvider key,
    ImageDecoderCallback decode,
    Future<void> abandoned, {
    required void Function(_SharedFetch) onFetch,
  }) async {
    var wasAbandoned = false;
    try {
      // Each decode size is its own cache entry (see [ResizeImage]), so one
      // URL shown at two sizes resolves twice. Share the download instead of
      // fetching the same bytes once per size.
      final fetchKey = (url, auth, isThumbnailFallback);
      final fetch = _inFlight[fetchKey] ??= _SharedFetch(
        _loadBytes,
        onFinished: (fetch) {
          if (_inFlight[fetchKey] == fetch) _inFlight.remove(fetchKey);
        },
      );
      fetch.join();
      onFetch(fetch);
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
      return withBrowserFrameTiming(await decode(buffer));
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
  /// saved once they decode). Stale saved bytes are shown, and once they
  /// decode a background download replaces them; saved bytes that fail to
  /// decode are removed instead.
  Future<Uint8List> _loadBytes(_SharedFetch fetch) async {
    final cache = diskCache;
    if (cache == null) return _fetchBytes(fetch.aborted);
    final hit = await cache.read(url);
    if (hit != null) {
      // Refresh only after the decode, so a refresh can never land before
      // the removal of bytes that failed it.
      if (hit.isStale) fetch.onDecoded = () => _scheduleRefresh(cache);
      fetch.onFailed = () => unawaited(cache.remove(url));
      return hit.bytes;
    }
    final bytes = await _fetchBytes(fetch.aborted);
    final immutable = _isContentAddressed;
    fetch.onDecoded = () =>
        unawaited(cache.write(url, bytes, immutable: immutable));
    return bytes;
  }

  void _scheduleRefresh(MediaDiskCache cache) {
    final key = (cache, url);
    if (_refreshQueue.length >= _maxQueuedRefreshes || !_refreshing.add(key)) {
      return;
    }
    _refreshQueue.add(() async {
      try {
        await _refresh(cache);
      } finally {
        _refreshing.remove(key);
      }
    });
    _pumpRefreshes();
  }

  static void _pumpRefreshes() {
    while (_runningRefreshes < _maxRefreshes && _refreshQueue.isNotEmpty) {
      final refresh = _refreshQueue.removeFirst();
      _runningRefreshes++;
      unawaited(
        refresh().whenComplete(() {
          _runningRefreshes--;
          _pumpRefreshes();
        }),
      );
    }
  }

  Future<void> _refresh(MediaDiskCache cache) async {
    try {
      // Nobody is waiting on it, so it stops only when its cache retires.
      if (cache.isClosed) return;
      final bytes = await _fetchBytes(cache.closed);
      // Nothing displays these bytes yet, so check they decode before they
      // replace the saved copy. A tiny target keeps the check from
      // allocating a full-resolution frame.
      try {
        final codec = await ui.instantiateImageCodec(
          bytes,
          targetWidth: 1,
          targetHeight: 1,
          allowUpscaling: false,
        );
        try {
          (await codec.getNextFrame()).image.dispose();
        } finally {
          codec.dispose();
        }
      } on Object {
        // A body that never decodes is a failed fetch: back off.
        _cooldownUntil[_cooldownKey] = debugNow().add(_defaultCooldown);
        rethrow;
      }
      await cache.write(url, bytes, immutable: _isContentAddressed);
    } on Object catch (error) {
      // The stale bytes stay; the next load after the cooldown retries.
      debugPrint('[MediaImageProvider] refresh of $url failed: $error');
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
    final until = _cooldownUntil[_cooldownKey];
    if (until != null) {
      if (debugNow().isBefore(until)) {
        throw MediaImageCooldownException(url: url, until: until);
      }
      _cooldownUntil.remove(_cooldownKey);
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
        _cooldownUntil[_cooldownKey] = debugNow().add(_defaultCooldown);
      }
      rethrow;
    }
    if (response.statusCode == 404 && isThumbnailFallback) {
      // Older or externally populated media may have no generated thumbnail.
      // Record that here rather than in a widget's error builder: the widget
      // that asked may be gone by the time a slow 404 arrives, and a
      // cooldown would then keep every later build off the next thumbnail.
      MediaImage._rememberMissingThumbnail(url);
      throw NetworkImageLoadException(statusCode: 404, uri: uri);
    }
    if (response.statusCode != 200) {
      _cooldownUntil[_cooldownKey] = debugNow().add(_cooldownFor(response));
      throw NetworkImageLoadException(
        statusCode: response.statusCode,
        uri: uri,
      );
    }
    final bytes = response.bytes;
    if (bytes.isEmpty) {
      _cooldownUntil[_cooldownKey] = debugNow().add(_defaultCooldown);
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
        other.auth == auth &&
        other.isThumbnailFallback == isThumbnailFallback;
  }

  @override
  int get hashCode => Object.hash(url, scale, auth, isThumbnailFallback);

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
    required this.onSettled,
    super.debugLabel,
  });

  /// Called once: with true when the first frame has decoded, or with false
  /// when the load fails before it. A codec can be created from bytes whose
  /// frames then fail to decode.
  void Function(bool decoded)? onSettled;

  void _settle({required bool decoded}) {
    final settled = onSettled;
    onSettled = null;
    settled?.call(decoded);
  }

  final _listeners = <ImageStreamListener>[];

  @override
  void setImage(ImageInfo image) {
    _settle(decoded: true);
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
    _settle(decoded: false);
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

/// Shows animation frames that ask for 10 ms or less for 100 ms instead, as
/// browsers do (Firefox first, then Chromium and WebKit), so an animated
/// emoji plays at the same speed as on desktop. Flutter uses the delay as
/// written, so a GIF with 0 or 1 centisecond delays otherwise runs at the
/// display refresh rate.
@visibleForTesting
ui.Codec withBrowserFrameTiming(ui.Codec codec) =>
    codec.frameCount > 1 ? _BrowserFrameTimingCodec(codec) : codec;

class _BrowserFrameTimingCodec implements ui.Codec {
  _BrowserFrameTimingCodec(this._codec);

  final ui.Codec _codec;

  @override
  int get frameCount => _codec.frameCount;

  @override
  int get repetitionCount => _codec.repetitionCount;

  @override
  Future<ui.FrameInfo> getNextFrame() async {
    final frame = await _codec.getNextFrame();
    if (frame.duration > const Duration(milliseconds: 10)) return frame;
    return _SlowedFrame(frame.image);
  }

  @override
  void dispose() => _codec.dispose();
}

class _SlowedFrame implements ui.FrameInfo {
  _SlowedFrame(this.image);

  @override
  final ui.Image image;

  @override
  Duration get duration => const Duration(milliseconds: 100);
}

/// One download shared by every load of the same URL, auth scope and role
/// (direct or thumbnail fallback). It is aborted once every load waiting for
/// it has been abandoned, and a load that arrives after that starts a new
/// download.
class _SharedFetch {
  _SharedFetch(
    Future<Uint8List> Function(_SharedFetch fetch) load, {
    required this.onFinished,
  }) {
    bytes = load(this).whenComplete(() {
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

  /// Completes when every load waiting for this download was abandoned.
  Future<void> get aborted => _abort.future;

  /// Run by the first load that decodes a frame: saves downloaded bytes,
  /// or refreshes stale saved ones.
  VoidCallback? onDecoded;

  /// Run when a load fails before decoding a frame: removes saved bytes.
  VoidCallback? onFailed;

  /// Runs [onDecoded] or [onFailed] once, by whether a load decoded a frame.
  void settle({required bool decoded}) {
    final pending = decoded ? onDecoded : onFailed;
    onDecoded = null;
    onFailed = null;
    pending?.call();
  }

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
  r'^/media/[0-9a-f]{64}(?:\.thumb\.(?:jpg|webp)|\.[a-z0-9]{1,8})?$',
);

final _relayBlobPath = RegExp(r'^/media/([0-9a-f]{64})(?:\.[a-z0-9]{1,8})?$');

/// The relay's generated thumbnails for a relay-hosted image, best first, or
/// empty when [url] is not one. Relays write `/media/<sha256>.thumb.webp`,
/// which keeps transparency, beside the `.thumb.jpg` that every relay
/// writes. Older relays, and images uploaded before WebP thumbnails, have
/// only the JPEG. Both are at most 320px on their longer side and still, so
/// animation is lost. This matches desktop's small avatars (buzz-app
/// `mediaUrl`).
List<String> relayThumbnailUrls(String url, MediaGetAuthService auth) {
  if (!auth.isRelayMediaUrl(url)) return const [];
  final uri = Uri.parse(url);
  if (uri.hasQuery || uri.hasFragment) return const [];
  final match = _relayBlobPath.firstMatch(uri.path);
  if (match == null) return const [];
  return [
    for (final ext in ['webp', 'jpg'])
      uri.replace(path: '/media/${match[1]}.thumb.$ext').toString(),
  ];
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
/// to the JPEG thumbnail when the relay has no WebP one. It never falls back
/// to the original: with no thumbnail it shows [errorBuilder], so a small
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
    MediaImageProvider providerFor(
      String source, {
      bool isThumbnailFallback = false,
    }) => MediaImageProvider(
      url: source,
      auth: auth,
      client: client,
      diskCache: diskCache,
      onBytesLoaded: onBytesLoaded,
      isThumbnailFallback: isThumbnailFallback,
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
        final thumbnails =
            preferThumbnail && _fitsThumbnail(context, constraints)
            ? relayThumbnailUrls(url, auth)
            : const <String>[];
        Widget source(Iterable<String> candidates) {
          final rest = candidates
              .skipWhile(_missingThumbnails.contains)
              .toList();
          if (rest.isEmpty) {
            if (thumbnails.isEmpty) return _image(providerFor(url), cacheWidth);
            return errorBuilder!(
              context,
              NetworkImageLoadException(
                statusCode: 404,
                uri: Uri.parse(thumbnails.last),
              ),
              null,
            );
          }
          return _image(
            providerFor(rest.first, isThumbnailFallback: true),
            cacheWidth,
            errorBuilder: (context, error, stackTrace) {
              if (error is! NetworkImageLoadException ||
                  error.statusCode != 404 ||
                  rest.length == 1) {
                return errorBuilder!(context, error, stackTrace);
              }
              // The fetch already recorded the missing thumbnail, so later
              // builds go straight to the next source.
              return source(rest.skip(1));
            },
          );
        }

        return source(thumbnails);
      },
    );
  }

  /// Thumbnails the relay returned 404 for, oldest first. Bounded so a
  /// community full of pre-thumbnail media cannot grow it without limit; an
  /// evicted entry costs one more 404 before the next source loads again.
  static final Set<String> _missingThumbnails = <String>{};
  static const _maxMissingThumbnails = 256;

  /// Called by the fetch that got the 404, which puts no failure cooldown on
  /// the thumbnail: this set stands in for it, and an entry evicted here then
  /// costs one more 404 rather than showing the fallback.
  static void _rememberMissingThumbnail(String url) {
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
    final image = ResizeImage.resizeIfNeeded(cacheWidth, null, provider);
    return _AbandonWhenGone(
      image: image,
      child: Image(
        image: image,
        fit: fit,
        width: width,
        height: height,
        semanticLabel: semanticLabel,
        errorBuilder: errorBuilder ?? this.errorBuilder,
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
