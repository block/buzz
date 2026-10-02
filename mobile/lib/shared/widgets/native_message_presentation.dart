import 'package:flutter/foundation.dart';
import 'package:flutter/services.dart';

/// UIKit presentation for message actions and reaction membership on iOS.
/// A null response means unavailable; a dismissed native surface is handled.
class NativeMessagePresentation {
  /// Whether the current platform can host UIKit.
  static bool get isSupportedPlatform =>
      !kIsWeb && defaultTargetPlatform == TargetPlatform.iOS;

  /// Transport shared with the iOS presentation coordinator.
  static const channel = MethodChannel('buzz/native_message_presentation');

  static final _presentedCallbacks = <String, VoidCallback>{};
  static var _nextRequest = 0;
  static bool _active = false;

  /// Owns preflight through dismissal across message and reaction surfaces.
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
    VoidCallback? onPresented,
  }) async {
    if (!isSupportedPlatform) return null;
    final requestId = 'message-${_nextRequest++}';
    if (onPresented != null) {
      _presentedCallbacks[requestId] = onPresented;
      channel.setMethodCallHandler((call) async {
        if (call.method == 'messagePresented') {
          _presentedCallbacks[call.arguments]?.call();
        }
      });
    }
    try {
      return await channel.invokeMapMethod<Object?, Object?>(method, {
        ...arguments,
        if (onPresented != null) 'requestId': requestId,
      });
    } on MissingPluginException {
      return null;
    } on PlatformException {
      return null;
    } finally {
      _presentedCallbacks.remove(requestId);
    }
  }
}
