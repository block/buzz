import 'package:flutter/foundation.dart';
import 'package:flutter/services.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';

/// Native side: `BuzzAppClipInviteBridge` in `ios/Runner/AppClipInviteBridge.swift`.
const _channel = MethodChannel('buzz/app_clip_invite');

/// Pulls the invite the iOS App Clip parked for this install, if any.
///
/// The native call reads and clears the shared Keychain / App Group record in
/// one step, so a returned link is delivered exactly once. The URL comes back
/// unparsed: callers run it through [parseBuzzDeepLink] like any universal
/// link. Platforms without the bridge resolve to `null`.
Future<Uri?> takePendingAppClipInvite() async {
  if (kIsWeb) return null;
  try {
    final raw = await _channel.invokeMethod<String>('takePendingInvite');
    if (raw == null || raw.isEmpty) return null;
    return Uri.tryParse(raw);
  } on MissingPluginException {
    return null;
  } on PlatformException catch (error) {
    // The record was dropped natively; nothing durable is left to retry.
    debugPrint('app-clip-invite: pickup failed: ${error.code}');
    return null;
  }
}

/// Startup pickup seam for the App Clip invite, overridable in tests.
final appClipInvitePickupProvider = Provider<Future<Uri?> Function()>(
  (ref) => takePendingAppClipInvite,
);
