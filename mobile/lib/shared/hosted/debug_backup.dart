import 'package:flutter/foundation.dart';

import 'hosted_environment.dart';

/// Temporary walkthrough simulation. Never enabled in profile/release or production.
abstract final class DebugBackup {
  static bool get enabled =>
      kDebugMode &&
      const bool.fromEnvironment('BUZZ_DEBUG_FAKE_BACKUP') &&
      HostedEnvironment.origin.host == 'app.builderlabstaging.xyz';

  static const receiptPrefix = 'debug-simulated-backup:';
}
