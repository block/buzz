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

  /// Completes after dismissal so callers can safely open the next surface.
  static Future<Map<Object?, Object?>?> present(
    String method,
    Map<String, Object?> arguments,
  ) async {
    if (!isSupportedPlatform) return null;
    try {
      return await channel.invokeMapMethod<Object?, Object?>(method, arguments);
    } on MissingPluginException {
      return null;
    } on PlatformException {
      return null;
    }
  }
}
