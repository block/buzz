import 'dart:async';
import 'dart:io';

import 'package:flutter/material.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';
import 'package:package_info_plus/package_info_plus.dart';
import 'package:shared_preferences/shared_preferences.dart';

import 'app.dart';
import 'features/age_gate/age_signal_push_bootstrap.dart';
import 'features/agents/agents_page.dart';
import 'features/invites/invite_join_provider.dart';
import 'shared/agents/agents_surface.dart';
import 'shared/push/push_bridge.dart';
import 'shared/relay/media_disk_cache.dart';
import 'shared/relay/relay_closed_policy.dart';
import 'shared/relay/relay_websocket.dart';
import 'shared/theme/theme_provider.dart';

void main() => runBuzzApp(const App());

Future<void> runBuzzApp(Widget app) async {
  WidgetsFlutterBinding.ensureInitialized();
  installBuzzPushMethodHandler();
  // Relay upgrades name the app version, so learn it before any socket opens.
  final userAgent = loadRelayUserAgent(
    () async => (await PackageInfo.fromPlatform()).version,
  );

  // Pre-load preferences so the first frame uses the saved theme/accent.
  final prefs = await SharedPreferences.getInstance();
  await userAgent;

  runApp(
    buildRootProviderScope(
      prefs: prefs,
      mediaCacheRoot: _mediaCacheRoot,
      child: app,
    ),
  );
}

/// Without a caches directory images still load, just not from disk.
Future<Directory?> _mediaCacheRoot() async {
  try {
    return await resolveMediaCacheRoot();
  } catch (error) {
    debugPrint('[MediaDiskCache] no caches directory: $error');
    return null;
  }
}

/// The app's root scope, shared with tests so its production wiring is
/// asserted rather than re-declared.
ProviderScope buildRootProviderScope({
  required SharedPreferences prefs,
  required Widget child,
  Future<Directory?> Function()? mediaCacheRoot,
}) => ProviderScope(
  // A relay statement deadline cannot resolve on retry — the same expensive
  // query would re-run and hit the same limit. Return null (no retry) for
  // deadline errors and fall through to the default exponential backoff for
  // everything else.
  retry: relayProviderRetry,
  overrides: [
    agentCreationSurfaceProvider.overrideWithValue(
      (context, link) async => unawaited(showAgents(context, draft: link)),
    ),
    savedPrefsProvider.overrideWithValue(prefs),
    if (mediaCacheRoot != null)
      mediaCacheRootProvider.overrideWith((ref) => mediaCacheRoot()),
    inviteJoinRecoveryProvider.overrideWith(
      (ref) =>
          (scope) => buildMobileInviteJoinRecovery(ref, scope),
    ),
  ],
  child: AgeSignalPushBootstrap(child: child),
);
