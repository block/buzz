import 'package:flutter/foundation.dart';
import 'package:flutter/services.dart';

/// The outcome communicated by a notice, independent of its wording.
enum ToastKind { generic, success, error }

/// Plays outcome-specific feedback without blocking the notice.
Future<void> toastHaptic(ToastKind kind) async {
  try {
    if (kind != ToastKind.generic &&
        !kIsWeb &&
        defaultTargetPlatform == TargetPlatform.iOS) {
      await const MethodChannel('buzz/haptics').invokeMethod<void>(kind.name);
      return;
    }
    switch (kind) {
      case ToastKind.generic:
        await HapticFeedback.lightImpact();
      case ToastKind.success:
        await HapticFeedback.mediumImpact();
      case ToastKind.error:
        await HapticFeedback.heavyImpact();
    }
  } on PlatformException {
    // Haptics are optional; feedback must still be displayed.
  } on MissingPluginException {
    // Some platforms and test embedders do not provide haptics.
  }
}
