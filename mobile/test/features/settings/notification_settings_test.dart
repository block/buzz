import 'package:buzz/features/settings/settings_page.dart';
import 'package:buzz/shared/notifications/notifications.dart';
import 'package:buzz/shared/relay/relay.dart';
import 'package:buzz/shared/theme/theme.dart';
import 'package:flutter/foundation.dart';
import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:shared_preferences/shared_preferences.dart';

import '../../helpers/widget_helpers.dart';

void main() {
  TestWidgetsFlutterBinding.ensureInitialized();

  late TargetPlatform? previousPlatform;

  setUp(() {
    previousPlatform = debugDefaultTargetPlatformOverride;
    SharedPreferences.setMockInitialValues({});
  });

  tearDown(() {
    debugDefaultTargetPlatformOverride = previousPlatform;
  });

  testWidgets('Android shows honest connection-bound notification copy', (
    tester,
  ) async {
    debugDefaultTargetPlatformOverride = TargetPlatform.android;
    try {
      final prefs = await SharedPreferences.getInstance();
      await tester.pumpWidget(
        WidgetHelpers.testable(
          child: const SettingsPage(profileHeader: SizedBox.shrink()),
          overrides: [
            savedPrefsProvider.overrideWithValue(prefs),
            relayConfigProvider.overrideWith(
              () => _RelayConfigNotifier('https://relay.example'),
            ),
            relaySessionProvider.overrideWith(
              () => _Session(SessionStatus.reconnecting),
            ),
            notificationSettingsProvider.overrideWith(
              () => _SettingsNotifier(
                const NotificationSettingsState(
                  alertsEnabled: true,
                  permission: AndroidNotificationPermission.granted,
                ),
              ),
            ),
          ],
        ),
      );
      await tester.pump();

      expect(find.text('Notifications'), findsOneWidget);
      expect(find.text('Paused'), findsOneWidget);
      expect(find.text('Background alerts'), findsOneWidget);
      expect(find.text('Message previews'), findsOneWidget);
      expect(find.text('Not available'), findsOneWidget);
      expect(
        find.text(
          'Buzz does not use Google services. Alerts require a live Buzz connection.',
        ),
        findsOneWidget,
      );
    } finally {
      debugDefaultTargetPlatformOverride = previousPlatform;
    }
  });

  testWidgets('non-Android settings remain unaffected', (tester) async {
    debugDefaultTargetPlatformOverride = TargetPlatform.iOS;
    try {
      final prefs = await SharedPreferences.getInstance();
      await tester.pumpWidget(
        WidgetHelpers.testable(
          child: const SettingsPage(profileHeader: SizedBox.shrink()),
          overrides: [
            savedPrefsProvider.overrideWithValue(prefs),
            relayConfigProvider.overrideWith(
              () => _RelayConfigNotifier('https://relay.example'),
            ),
            relaySessionProvider.overrideWith(
              () => _Session(SessionStatus.connected),
            ),
          ],
        ),
      );
      await tester.pump();

      expect(find.text('Notifications'), findsNothing);
      expect(find.text('Background alerts'), findsNothing);
    } finally {
      debugDefaultTargetPlatformOverride = previousPlatform;
    }
  });
}

class _RelayConfigNotifier extends RelayConfigNotifier {
  _RelayConfigNotifier(this.url);

  final String url;

  @override
  RelayConfig build() => RelayConfig(baseUrl: url);
}

class _Session extends RelaySessionNotifier {
  _Session(this.status);

  final SessionStatus status;

  @override
  SessionState build() => SessionState(status: status);
}

class _SettingsNotifier extends NotificationSettingsNotifier {
  _SettingsNotifier(this.initialState);

  final NotificationSettingsState initialState;

  @override
  NotificationSettingsState build() => initialState;
}
