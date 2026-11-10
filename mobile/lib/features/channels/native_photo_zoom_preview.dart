import 'dart:async';
import 'dart:ui' as ui;

import 'package:flutter/foundation.dart';
import 'package:flutter/material.dart';
import 'package:flutter/services.dart';

import '../../shared/relay/media_image.dart';

/// Message-scoped actions shared with the Flutter gallery.
class NativePhotoActions {
  const NativePhotoActions({required this.canDelete, required this.perform});
  final bool canDelete;
  final Future<String?> Function(String action, String url) perform;
}

/// UIKit gallery presentation, enabled in the photo-stack preview.
class NativePhotoZoomPreview {
  static const enabled = bool.fromEnvironment('BUZZ_NATIVE_PHOTO_ZOOM');
  static const channel = MethodChannel('buzz/native_photo_zoom');
  static bool _presenting = false;
  static int _session = 0;

  /// Returns true only when the existing Flutter gallery is needed as fallback.
  ///
  /// Only JPEG galleries use the static PNG bridge. Animation-capable formats
  /// (including ordinary PNGs) and unknown types keep the Flutter renderer.
  /// [mimeTypes], when provided, contains one declared type per [urls] entry.
  static Future<bool> open({
    required BuildContext context,
    required List<GlobalKey> sourceKeys,
    required List<ImageProvider<Object>> providers,
    required List<String> urls,
    List<String?>? mimeTypes,
    required List<String> labels,
    ImageProvider<Object>? previewProvider,
    List<ImageProvider<Object>>? previewProviders,
    required int initialIndex,
    required Future<void> Function(int) onSelect,
    VoidCallback? onReply,
    NativePhotoActions? actions,
  }) async {
    if (!enabled || defaultTargetPlatform != TargetPlatform.iOS) return true;
    // PNG/APNG, WebP, GIF and unknown formats may animate. The PNG bridge
    // cannot preserve their frames, so keep the whole gallery in Flutter.
    // Metadata wins over the extension (e.g. a GIF served at a .jpg URL).
    for (var i = 0; i < urls.length; i++) {
      final mime = mimeTypes?[i]?.split(';').first.trim().toLowerCase();
      final path = Uri.tryParse(urls[i])?.path.toLowerCase() ?? '';
      if (mime != null
          ? mime != 'image/jpeg'
          : !(path.endsWith('.jpg') || path.endsWith('.jpeg'))) {
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
      final loads = _PhotoWindow(configuration, initialIndex);
      window = loads;
      final bytes = await loads.load(
        initialIndex,
        previewProvider ??
            previewProviders?[initialIndex] ??
            providers[initialIndex],
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
          case 'preview':
            final provider = previewProviders?[index];
            return provider == null
                ? null
                : loads.load(index, provider, preview: true);
          case 'image':
            return loads.load(index, providers[index]);
          case 'select':
            loads.select(index);
            await onSelect(index).timeout(const Duration(seconds: 2));
            if (!context.mounted) return null;
            return _geometry(sourceKeys[index]);
          case 'action':
            final action = args['action'];
            if (actions == null ||
                !['save', 'share', 'copy', 'delete'].contains(action)) {
              return null;
            }
            if (action == 'delete' && !actions.canDelete) return null;
            return actions.perform(action as String, urls[index]);
        }
        return null;
      });
      final result = await channel.invokeMethod<String>('present', {
        ...geometry,
        'bytes': bytes,
        'session': session,
        'labels': labels,
        'initialIndex': initialIndex,
        'canReply': onReply != null,
        'canMore': actions != null,
        'canDelete': actions?.canDelete ?? false,
        'reduceMotion': reduceMotion,
      });
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
  _PhotoWindow(this.configuration, this.index);
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
  _PhotoLoad(this.provider, this.configuration) {
    stream = provider.resolve(configuration);
    listener = ImageStreamListener(
      (info, _) {
        receivedFrame = true;
        if (!frame.isCompleted) {
          frame.complete(info);
        } else {
          info.dispose();
        }
      },
      onError: (Object _, StackTrace? _) {
        if (!frame.isCompleted) frame.complete(null);
      },
    );
    stream.addListener(listener);
    timer = Timer(const Duration(seconds: 10), dispose);
    result = encode();
  }

  final ImageProvider<Object> provider;
  final ImageConfiguration configuration;
  final frame = Completer<ImageInfo?>();
  late final ImageStream stream;
  late final ImageStreamListener listener;
  late final Timer timer;
  late final Future<Uint8List?> result;
  bool disposed = false;
  bool detached = false;
  bool receivedFrame = false;

  Future<Uint8List?> encode() async {
    final info = await frame.future;
    detach();
    try {
      if (info == null || disposed) return null;
      final data = await info.image.toByteData(format: ui.ImageByteFormat.png);
      if (disposed || data == null) return null;
      return data.buffer.asUint8List(data.offsetInBytes, data.lengthInBytes);
    } catch (_) {
      return null;
    } finally {
      info?.dispose();
      timer.cancel();
    }
  }

  void detach() {
    if (detached) return;
    detached = true;
    stream.removeListener(listener);
    if (!receivedFrame) abandonPendingImage(stream, provider, configuration);
  }

  void dispose() {
    disposed = true;
    timer.cancel();
    if (!frame.isCompleted) frame.complete(null);
    detach();
  }
}
