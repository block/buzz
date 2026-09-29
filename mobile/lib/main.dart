import 'package:flutter/material.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';
import 'package:shared_preferences/shared_preferences.dart';

import 'app.dart';
import 'features/age_gate/age_signal_push_bootstrap.dart';
import 'features/invites/invite_join_provider.dart';
import 'shared/push/push_bridge.dart';
import 'shared/relay/relay_closed_policy.dart';
import 'shared/theme/theme_provider.dart';

void main() => runBuzzApp(const App());

Future<void> runBuzzApp(Widget app) async {
  WidgetsFlutterBinding.ensureInitialized();
  installBuzzPushMethodHandler();
  await syncPendingBuzzPushNotificationResponse();

  // Pre-load preferences so the first frame uses the saved theme/accent.
  final prefs = await SharedPreferences.getInstance();

  runApp(buildRootProviderScope(prefs: prefs, child: app));
}

/// The app's root scope, shared with tests so its production wiring is
/// asserted rather than re-declared.
ProviderScope buildRootProviderScope({
  required SharedPreferences prefs,
  required Widget child,
}) => ProviderScope(
  // A relay statement deadline cannot resolve on retry — the same expensive
  // query would re-run and hit the same limit. Return null (no retry) for
  // deadline errors and fall through to the default exponential backoff for
  // everything else.
  retry: relayProviderRetry,
  overrides: [
    savedPrefsProvider.overrideWithValue(prefs),
    inviteJoinRecoveryProvider.overrideWith(
      (ref) =>
          (scope) => buildMobileInviteJoinRecovery(ref, scope),
    ),
  ],
  child: AgeSignalPushBootstrap(child: child),
);
