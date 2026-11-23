import 'package:flutter/foundation.dart';
import 'package:flutter/services.dart';

/// UIKit presentation for reaction membership on iOS.
/// A null response means unavailable; a dismissed native surface is handled.
class NativeMessagePresentation {
  /// Whether the current platform can host UIKit.
  static bool get isSupportedPlatform =>
      !kIsWeb && defaultTargetPlatform == TargetPlatform.iOS;

  /// Transport shared with the iOS presentation coordinator.
  static const channel = MethodChannel('buzz/native_message_presentation');

  static var _nextRequest = 0;
  static bool _active = false;
  static String? _reactionRequest;

  /// Owns preflight through dismissal across reaction surfaces.
  /// An overlapping request is consumed instead of opening a fallback surface.
  static Future<bool> withLease(Future<bool> Function() presentation) async {
    if (_active) return true;
    _active = true;
    try {
      return await presentation();
    } finally {
      _active = false;
    }
  }

  /// Completes after dismissal so callers can safely open the next surface.
  static Future<Map<Object?, Object?>?> present(
    String method,
    Map<String, Object?> arguments, {
    ValueChanged<String>? onReaction,
  }) async {
    if (!isSupportedPlatform) return null;
    final requestId =
        arguments['requestId'] as String? ?? 'message-${_nextRequest++}';
    if (onReaction != null) {
      _reactionRequest = requestId;
      channel.setMethodCallHandler((call) async {
        if (call.method == 'toggleReaction') {
          final data = call.arguments as Map;
          if (data['requestId'] == requestId && data['emoji'] is String) {
            onReaction.call(data['emoji'] as String);
          }
        }
      });
    }
    try {
      return await channel.invokeMapMethod<Object?, Object?>(method, {
        ...arguments,
        'requestId': requestId,
      });
    } on MissingPluginException {
      return null;
    } on PlatformException {
      return null;
    } finally {
      if (onReaction != null) {
        if (_reactionRequest == requestId) _reactionRequest = null;
        channel.setMethodCallHandler(null);
      }
    }
  }

  /// Shows [message] on the open reaction sheet, which covers Flutter's own
  /// overlay. False when no reaction sheet is open to show it.
  static Future<bool> showNotice(String message) async {
    final requestId = _reactionRequest;
    if (requestId == null) return false;
    try {
      await channel.invokeMethod<void>('notice', {
        'requestId': requestId,
        'message': message,
      });
      return true;
    } on MissingPluginException {
      return false;
    } on PlatformException {
      return false;
    }
  }
}
