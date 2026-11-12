import 'dart:async';
import 'dart:ui' as ui;

import 'package:flutter/foundation.dart';
import 'package:flutter/material.dart';
import 'package:flutter/rendering.dart';
import 'package:flutter/services.dart';

import '../../shared/relay/media_image.dart';
import '../../shared/widgets/ios_navigation_metrics.dart';

/// Message-scoped actions shared with the Flutter gallery.
class NativePhotoActions {
  const NativePhotoActions({required this.canDelete, required this.perform});
  final bool canDelete;
  final Future<String?> Function(String action, String url) perform;
}

/// UIKit gallery presentation with the Flutter viewer as a fallback.
class NativePhotoZoomPreview {
  static const enabled = bool.fromEnvironment(
    'BUZZ_NATIVE_PHOTO_ZOOM',
    defaultValue: true,
  );
  static const channel = MethodChannel('buzz/native_photo_zoom');
  static bool _presenting = false;
  static int _session = 0;

  /// Returns true only when the existing Flutter gallery is needed as fallback.
  ///
  /// JPEG, PNG and HEIC/HEIF photos use the native gallery. Continuing frames
  /// are forwarded for animated PNGs; other formats keep the Flutter renderer.
  /// [mimeTypes], when provided, contains one declared type per [urls] entry.
  /// [videoIndices] use posters (or source snapshots) and [loadVideo] local files;
  /// their URLs never enter the image decoder.
  static Future<bool> open({
    required BuildContext context,
    required List<GlobalKey> sourceKeys,
    required List<ImageProvider<Object>> providers,
    required List<String> urls,
    List<String?>? mimeTypes,
    required List<String> labels,
    ImageProvider<Object>? previewProvider,
    List<ImageProvider<Object>?>? previewProviders,
    required int initialIndex,
    required Future<void> Function(int) onSelect,
    VoidCallback? onReply,
    NativePhotoActions? actions,
    Set<int> videoIndices = const {},
    Future<String?> Function(int)? loadVideo,
  }) async {
    if (!enabled || defaultTargetPlatform != TargetPlatform.iOS) return true;
    // Metadata wins over the extension (e.g. a GIF served at a .jpg URL).
    for (var i = 0; i < urls.length; i++) {
      if (videoIndices.contains(i)) continue;
      final mime = mimeTypes?[i]?.split(';').first.trim().toLowerCase();
      final path = Uri.tryParse(urls[i])?.path.toLowerCase() ?? '';
      if (mime != null
          ? !const {
              'image/jpeg',
              'image/png',
              'image/apng',
              'image/heic',
              'image/heif',
            }.contains(mime)
          : !const [
              '.jpg',
              '.jpeg',
              '.png',
              '.apng',
              '.heic',
              '.heif',
            ].any(path.endsWith)) {
        return true;
      }
    }
    if (_presenting) return false;
    _presenting = true;
    final session = ++_session;
    _PhotoWindow? window;
    try {
      if (await channel.invokeMethod<bool>('isSupported') != true) return true;
      if (!context.mounted) return false;
      final configuration = createLocalImageConfiguration(context);
      final reduceMotion = MediaQuery.disableAnimationsOf(context);
      final railMetrics = IosNavigationMetrics.of(context);
      final loads = _PhotoWindow(configuration, initialIndex, (
        index,
        bytes,
      ) async {
        if (!context.mounted) return;
        await channel.invokeMethod<void>('imageFrame', {
          'session': session,
          'index': index,
          'bytes': bytes,
        });
      });
      window = loads;
      final initialPoster = previewProvider ?? previewProviders?[initialIndex];
      final isVideo = videoIndices.contains(initialIndex);
      final posterLoad = isVideo && initialPoster != null
          ? loads.load(initialIndex, initialPoster, preview: true)
          : null;
      // Only wait for PNG encoding when the poster was already decoded.
      // Network-backed posters must never delay opening playable video.
      final cachedPoster =
          loads.loads[(initialIndex, true)]?.receivedFrame == true;
      final bytes = isVideo
          ? (cachedPoster ? await posterLoad : null) ??
                await _snapshot(sourceKeys[initialIndex])
          : await loads.load(
              initialIndex,
              initialPoster ?? providers[initialIndex],
              preview: true,
            );
      if (bytes == null) return true;
      if (!context.mounted) return false;
      final geometry = _geometry(sourceKeys[initialIndex]);
      if (geometry == null) return true;
      channel.setMethodCallHandler((call) async {
        final args = call.arguments;
        if (args is! Map || args['session'] != session || !context.mounted) {
          return null;
        }
        final index = args['index'];
        if (index is! int || index < 0 || index >= providers.length) {
          return null;
        }
        switch (call.method) {
          case 'video':
            if (!videoIndices.contains(index) || index != loads.index) {
              return null;
            }
            return loadVideo?.call(index);
          case 'preview':
            final provider = previewProviders?[index];
            if (videoIndices.contains(index)) {
              return (provider == null
                      ? null
                      : await loads.load(index, provider, preview: true)) ??
                  await _snapshot(sourceKeys[index]);
            }
            return provider == null
                ? null
                : loads.load(index, provider, preview: true);
          case 'image':
            if (videoIndices.contains(index)) return null;
            return loads.load(index, providers[index]);
          case 'select':
            loads.select(index);
            await onSelect(index).timeout(const Duration(seconds: 2));
            if (!context.mounted) return null;
            return _geometry(sourceKeys[index]);
          case 'action':
            final action = args['action'];
            if (videoIndices.contains(index) ||
                actions == null ||
                !['save', 'share', 'copy', 'delete'].contains(action)) {
              return null;
            }
            if (action == 'delete' && !actions.canDelete) return null;
            return actions.perform(action as String, urls[index]);
        }
        return null;
      });
      final presentation = channel.invokeMethod<String>('present', {
        ...geometry,
        'bytes': bytes,
        'session': session,
        'labels': labels,
        'videoIndices': videoIndices.toList(),
        'initialIndex': initialIndex,
        'canReply': onReply != null,
        'canMore': actions != null,
        'canDelete': actions?.canDelete ?? false,
        'reduceMotion': reduceMotion,
        'railCenterX': railMetrics.railCenterX,
        'railWindowWidth': railMetrics.windowSize?.width,
        'railWindowHeight': railMetrics.windowSize?.height,
      });
      if (posterLoad != null && !cachedPoster) {
        unawaited(
          posterLoad
              .then((poster) async {
                if (poster != null &&
                    !loads.disposed &&
                    loads.index == initialIndex) {
                  await loads.onFrame(initialIndex, poster);
                }
              })
              .catchError((Object _) {}),
        );
      }
      final result = await presentation;
      if (result == 'reply' && context.mounted) onReply?.call();
      return result == null || result == 'unavailable';
    } catch (_) {
      // Preserve the established gallery and its error handling on bridge failure.
      return context.mounted;
    } finally {
      window?.dispose();
      channel.setMethodCallHandler(null);
      _presenting = false;
    }
  }

  static Future<Uint8List?> _snapshot(GlobalKey key) async {
    final boundary = key.currentContext?.findRenderObject();
    if (boundary is! RenderRepaintBoundary || boundary.debugNeedsPaint) {
      return null;
    }
    final image = await boundary.toImage(pixelRatio: 1);
    try {
      final data = await image.toByteData(format: ui.ImageByteFormat.png);
      return data?.buffer.asUint8List();
    } finally {
      image.dispose();
    }
  }

  static Map<String, double>? _geometry(GlobalKey key) {
    final box = key.currentContext?.findRenderObject();
    if (box is! RenderBox ||
        !box.attached ||
        !box.hasSize ||
        box.size.isEmpty) {
      return null;
    }
    final center = box.localToGlobal(box.size.center(Offset.zero));
    final direction =
        box.localToGlobal(Offset(box.size.width, 0)) -
        box.localToGlobal(Offset.zero);
    return {
      'x': center.dx,
      'y': center.dy,
      'width': direction.distance,
      'height': box.size.height * direction.distance / box.size.width,
      'angle': direction.direction,
    };
  }
}

// At most five nearby photos, at preview and full resolution. Moving the window
// abandons stale downloads; closing also releases all listeners and PNG bytes.
class _PhotoWindow {
  _PhotoWindow(this.configuration, this.index, this.onFrame);
  final Future<void> Function(int, Uint8List) onFrame;
  final ImageConfiguration configuration;
  int index;
  bool disposed = false;
  final loads = <(int, bool), _PhotoLoad>{};

  Future<Uint8List?> load(
    int value,
    ImageProvider<Object> provider, {
    bool preview = false,
  }) {
    if (disposed || (value - index).abs() > 2) return Future.value(null);
    return (loads[(value, preview)] ??= _PhotoLoad(
      provider,
      configuration,
      shouldForwardFrame: () => !disposed && value == index,
      onFrame: preview
          ? null
          : (bytes) async {
              if (!disposed && value == index) await onFrame(value, bytes);
            },
    )).result;
  }

  void select(int value) {
    index = value;
    for (final key in loads.keys.toList()) {
      if ((key.$1 - value).abs() > 2) loads.remove(key)?.dispose();
    }
  }

  void dispose() {
    disposed = true;
    for (final load in loads.values) {
      load.dispose();
    }
    loads.clear();
  }
}

class _PhotoLoad {
  _PhotoLoad(
    this.provider,
    this.configuration, {
    this.onFrame,
    required this.shouldForwardFrame,
  }) {
    stream = provider.resolve(configuration);
    listener = ImageStreamListener((info, _) {
      receivedFrame = true;
      if (disposed || (first.isCompleted && !shouldForwardFrame())) {
        info.dispose();
        return;
      }
      nextFrame?.dispose();
      nextFrame = info;
      if (!encoding) unawaited(encode());
    }, onError: (Object _, StackTrace? _) => dispose());
    timer = Timer(const Duration(seconds: 10), dispose);
    stream.addListener(listener);
  }

  final ImageProvider<Object> provider;
  final ImageConfiguration configuration;
  final Future<void> Function(Uint8List)? onFrame;
  final bool Function() shouldForwardFrame;
  final first = Completer<Uint8List?>();
  Future<Uint8List?> get result => first.future;
  late final ImageStream stream;
  late final ImageStreamListener listener;
  late final Timer timer;
  ImageInfo? nextFrame;
  bool encoding = false;
  bool disposed = false;
  bool detached = false;
  bool receivedFrame = false;

  // One encoding and one latest frame per load, including while the platform
  // channel is busy. Never queue an animation's entire decoded frame history.
  Future<void> encode() async {
    encoding = true;
    try {
      while (!disposed && nextFrame != null) {
        final info = nextFrame!;
        nextFrame = null;
        try {
          final data = await info.image.toByteData(
            format: ui.ImageByteFormat.png,
          );
          if (disposed || data == null) {
            dispose();
            return;
          }
          final bytes = data.buffer.asUint8List(
            data.offsetInBytes,
            data.lengthInBytes,
          );
          if (!first.isCompleted) {
            first.complete(bytes);
            timer.cancel();
            if (onFrame == null) {
              dispose();
              return;
            }
          } else {
            await onFrame?.call(bytes);
          }
        } finally {
          info.dispose();
        }
      }
    } catch (_) {
      dispose();
    } finally {
      encoding = false;
    }
  }

  void detach() {
    if (detached) return;
    detached = true;
    stream.removeListener(listener);
    if (!receivedFrame) abandonPendingImage(stream, provider, configuration);
  }

  void dispose() {
    if (disposed) return;
    disposed = true;
    timer.cancel();
    if (!first.isCompleted) first.complete(null);
    nextFrame?.dispose();
    nextFrame = null;
    detach();
  }
}
