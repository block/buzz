import 'package:flutter/foundation.dart';
import 'package:flutter/services.dart';

/// Plays native success feedback after an action completes.
Future<void> successHaptic() async {
  try {
    if (!kIsWeb && defaultTargetPlatform == TargetPlatform.iOS) {
      await const MethodChannel('buzz/haptics').invokeMethod<void>('success');
    } else {
      await HapticFeedback.mediumImpact();
    }
  } on PlatformException {
    // Feedback availability must not affect the completed action.
  } on MissingPluginException {
    // Some embedders do not provide haptic feedback.
  }
}
