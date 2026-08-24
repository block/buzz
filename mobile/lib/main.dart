import 'package:flutter/material.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';
import 'package:shared_preferences/shared_preferences.dart';

import 'app.dart';
import 'shared/deeplink/pending_deep_link_provider.dart';
import 'shared/notifications/local_notifications_service.dart';
import 'shared/theme/theme_provider.dart';

Future<void> main() async {
  WidgetsFlutterBinding.ensureInitialized();

  // Pre-load preferences so the first frame uses the saved theme/accent.
  final prefs = await SharedPreferences.getInstance();

  final container = ProviderContainer(
    overrides: [savedPrefsProvider.overrideWithValue(prefs)],
  );

  await LocalNotificationsService.instance.initialize(
    onTap: (link) {
      container.read(pendingDeepLinkProvider.notifier).parkLink(link);
    },
  );

  runApp(UncontrolledProviderScope(container: container, child: const App()));
}
