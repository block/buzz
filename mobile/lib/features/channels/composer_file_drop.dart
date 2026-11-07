import 'dart:async';
import 'dart:io';

import 'package:flutter/foundation.dart';
import 'package:flutter/services.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';

/// A file the user dragged onto the app, copied into the app sandbox.
///
/// The receiving composer owns [path] and must delete it after use.
@immutable
class DroppedFile {
  final String path;
  final String name;
  final String mimeType;

  const DroppedFile({
    required this.path,
    required this.name,
    required this.mimeType,
  });
}

/// The result of one drag-and-drop operation.
@immutable
class ComposerDrop {
  final List<DroppedFile> files;

  /// Items the platform offered but could not copy.
  final int failures;

  const ComposerDrop({required this.files, this.failures = 0});
}

/// Routes native drag-and-drop file drops to the most recently mounted
/// composer. The platform only accepts drops while a composer is registered.
class ComposerFileDrop {
  ComposerFileDrop({MethodChannel? channel})
    : _channel = channel ?? const MethodChannel('buzz/composer_drop') {
    _channel.setMethodCallHandler(_handleCall);
  }

  final MethodChannel _channel;
  final _handlers = <ValueChanged<ComposerDrop>>[];

  /// Registers [handler] as the active drop target until the returned
  /// callback is invoked.
  VoidCallback register(ValueChanged<ComposerDrop> handler) {
    _handlers.add(handler);
    if (_handlers.length == 1) unawaited(_setEnabled(true));
    return () {
      if (!_handlers.remove(handler)) return;
      if (_handlers.isEmpty) unawaited(_setEnabled(false));
    };
  }

  Future<void> _setEnabled(bool enabled) async {
    try {
      await _channel.invokeMethod<void>('setEnabled', enabled);
    } on MissingPluginException {
      // Platforms without native drop support.
    } on PlatformException {
      // Drop support is best effort.
    }
  }

  @visibleForTesting
  Future<void> handleCall(MethodCall call) => _handleCall(call);

  Future<void> _handleCall(MethodCall call) async {
    if (call.method != 'didDrop') return;
    final drop = parseDrop(call.arguments);
    if (_handlers.isEmpty) {
      for (final file in drop.files) {
        await _deleteQuietly(file.path);
      }
      return;
    }
    _handlers.last(drop);
  }

  @visibleForTesting
  static ComposerDrop parseDrop(Object? arguments) {
    if (arguments is! Map) return const ComposerDrop(files: []);
    final rawFiles = arguments['files'];
    final failures = arguments['failures'];
    final files = <DroppedFile>[];
    if (rawFiles is List) {
      for (final raw in rawFiles) {
        if (raw is! Map) continue;
        final path = raw['path'];
        if (path is! String || path.isEmpty) continue;
        final name = raw['name'];
        final mimeType = raw['mimeType'];
        files.add(
          DroppedFile(
            path: path,
            name: name is String && name.isNotEmpty
                ? name
                : path.split(Platform.pathSeparator).last,
            mimeType: mimeType is String && mimeType.isNotEmpty
                ? mimeType
                : 'application/octet-stream',
          ),
        );
      }
    }
    return ComposerDrop(files: files, failures: failures is int ? failures : 0);
  }

  static Future<void> _deleteQuietly(String path) async {
    try {
      await File(path).delete();
    } on FileSystemException {
      // Already gone.
    }
  }
}

final composerFileDropProvider = Provider<ComposerFileDrop>(
  (ref) => ComposerFileDrop(),
);
