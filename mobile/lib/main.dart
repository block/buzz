import 'package:flutter/material.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';
import 'package:shared_preferences/shared_preferences.dart';

import 'app.dart';
import 'features/invites/invite_join_provider.dart';
import 'shared/push/push_bootstrap.dart';
import 'shared/push/push_bridge.dart';
import 'shared/theme/theme_provider.dart';

void main() => runBuzzApp(const App());

Future<void> runBuzzApp(Widget app) async {
  WidgetsFlutterBinding.ensureInitialized();
  installBuzzPushMethodHandler();
  await syncPendingBuzzPushNotificationResponse();

  // Pre-load preferences so the first frame uses the saved theme/accent.
  final prefs = await SharedPreferences.getInstance();

  runApp(
    ProviderScope(
      overrides: [
        savedPrefsProvider.overrideWithValue(prefs),
        inviteJoinRecoveryProvider.overrideWith(
          (ref) =>
              (scope) => buildMobileInviteJoinRecovery(ref, scope),
        ),
      ],
      child: BuzzPushBootstrap(child: app),
    ),
  );
}
