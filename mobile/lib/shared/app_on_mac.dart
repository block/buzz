import 'package:flutter/foundation.dart';
import 'package:flutter/services.dart';

const _platformInfoChannel = MethodChannel('buzz/platform_info');

bool _isIosAppOnMac = false;

/// True when this iOS build is running on an Apple silicon Mac ("Designed
/// for iPad"). There the Mac text system turns Return into a newline before
/// Flutter sees a key event, so the composer sends via the text input action.
bool get isIosAppOnMac => _isIosAppOnMac;

Future<void> loadIosAppOnMac() async {
  if (kIsWeb || defaultTargetPlatform != TargetPlatform.iOS) return;
  try {
    _isIosAppOnMac =
        await _platformInfoChannel.invokeMethod<bool>('isIosAppOnMac') ?? false;
  } on Object {
    _isIosAppOnMac = false;
  }
}

@visibleForTesting
set debugIsIosAppOnMac(bool value) => _isIosAppOnMac = value;
