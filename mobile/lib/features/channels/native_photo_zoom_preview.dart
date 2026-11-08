import 'dart:async';
import 'dart:ui' as ui;

import 'package:flutter/foundation.dart';
import 'package:flutter/material.dart';
import 'package:flutter/services.dart';

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
  static Future<bool> open({
    required BuildContext context,
    required List<GlobalKey> sourceKeys,
    required List<ImageProvider<Object>> providers,
    required List<String> urls,
    required List<String> labels,
    ImageProvider<Object>? previewProvider,
    List<ImageProvider<Object>>? previewProviders,
    required int initialIndex,
    required Future<void> Function(int) onSelect,
    VoidCallback? onReply,
    NativePhotoActions? actions,
  }) async {
    if (!enabled || defaultTargetPlatform != TargetPlatform.iOS) return true;
    if (_presenting) return false;
    _presenting = true;
    final session = ++_session;
    try {
      if (await channel.invokeMethod<bool>('isSupported') != true) return true;
      if (!context.mounted) return false;
      final configuration = createLocalImageConfiguration(context);
      final reduceMotion = MediaQuery.disableAnimationsOf(context);
      final bytes = await _imageBytes(
        previewProvider ?? providers[initialIndex],
        configuration,
      );
      // Prepare remaining previews without delaying the tapped photo's opening.
      final previews = <Future<Uint8List?>>[
        for (var i = 0; i < providers.length; i++)
          i == initialIndex
              ? Future.value(bytes)
              : previewProviders == null
              ? Future.value(null)
              : _imageBytes(
                  previewProviders[i],
                  configuration,
                ).then<Uint8List?>(
                  (value) => value,
                  onError: (Object _, StackTrace _) => null,
                ),
      ];
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
            return previews[index];
          case 'image':
            return _imageBytes(providers[index], configuration);
          case 'select':
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
        'previews': [
          for (var i = 0; i < providers.length; i++)
            i == initialIndex ? bytes : null,
        ],
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

  static Future<Uint8List> _imageBytes(
    ImageProvider<Object> provider,
    ImageConfiguration configuration,
  ) async {
    final completer = Completer<ImageInfo>();
    final stream = provider.resolve(configuration);
    final listener = ImageStreamListener(
      (info, _) {
        if (!completer.isCompleted) {
          completer.complete(info);
        } else {
          info.dispose();
        }
      },
      onError: (Object error, StackTrace? stack) {
        if (!completer.isCompleted) completer.completeError(error, stack);
      },
    );
    stream.addListener(listener);
    ImageInfo? frame;
    try {
      frame = await completer.future.timeout(const Duration(seconds: 10));
      final data = await frame.image.toByteData(format: ui.ImageByteFormat.png);
      if (data == null) throw StateError('Unable to encode photo');
      return data.buffer.asUint8List(data.offsetInBytes, data.lengthInBytes);
    } finally {
      stream.removeListener(listener);
      frame?.dispose();
    }
  }
}
