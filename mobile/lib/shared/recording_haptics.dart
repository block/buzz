import 'package:flutter/foundation.dart';
import 'package:flutter/services.dart';

const _channel = MethodChannel('buzz/haptics');

Future<bool> _native(String method) async {
  if (kIsWeb || defaultTargetPlatform != TargetPlatform.iOS) return false;
  try {
    await _channel.invokeMethod<void>(method);
    return true;
  } on PlatformException {
    return false;
  } on MissingPluginException {
    return false;
  }
}

/// Warms the iPhone feedback engine before the hold or cancel threshold.
Future<void> prepareRecordingHaptic({bool cancel = false}) async {
  await _native(cancel ? 'voicePrepareCancel' : 'voicePrepareStart');
}

/// Delivers recording feedback without allowing platform errors to stop capture.
Future<void> recordingHaptic({bool cancel = false}) async {
  if (await _native(cancel ? 'voiceCancel' : 'voiceStart')) return;
  try {
    if (cancel) {
      await HapticFeedback.selectionClick();
    } else {
      await HapticFeedback.mediumImpact();
    }
  } on PlatformException {
    // Recording must work even if feedback is unavailable.
  } on MissingPluginException {
    // Some embedders have no haptic engine.
  }
}
