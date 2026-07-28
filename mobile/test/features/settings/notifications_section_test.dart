import 'package:buzz/features/settings/settings_page.dart';
import 'package:buzz/shared/notifications/agent_live_update_preferences.dart';
import 'package:buzz/shared/relay/relay.dart';
import 'package:buzz/shared/theme/theme_provider.dart';
import 'package:flutter/foundation.dart';
import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';
import 'package:shared_preferences/shared_preferences.dart';

void main() {
  testWidgets('shows the agent activity switch on Android', (tester) async {
    final previousPlatform = debugDefaultTargetPlatformOverride;
    debugDefaultTargetPlatformOverride = TargetPlatform.android;
    try {
      SharedPreferences.setMockInitialValues({});
      final prefs = await SharedPreferences.getInstance();

      await tester.pumpWidget(
        ProviderScope(
          overrides: [
            savedPrefsProvider.overrideWithValue(prefs),
            relayConfigProvider.overrideWith(_FakeRelayConfigNotifier.new),
          ],
          child: const MaterialApp(
            home: SettingsPage(profileHeader: SizedBox.shrink()),
          ),
        ),
      );
      await tester.pump();

      expect(find.text('Notifications'), findsOneWidget);
      expect(find.text('Agent activity'), findsOneWidget);
      expect(find.byType(Switch), findsOneWidget);

      final container = ProviderScope.containerOf(
        tester.element(find.byType(SettingsPage)),
      );
      expect(container.read(agentLiveUpdatesEnabledProvider), isTrue);

      await tester.tap(find.byType(Switch));
      await tester.pump();

      expect(container.read(agentLiveUpdatesEnabledProvider), isFalse);
    } finally {
      debugDefaultTargetPlatformOverride = previousPlatform;
    }
  });

  testWidgets('shows the agent activity switch on iOS', (tester) async {
    final previousPlatform = debugDefaultTargetPlatformOverride;
    debugDefaultTargetPlatformOverride = TargetPlatform.iOS;
    try {
      SharedPreferences.setMockInitialValues({});
      final prefs = await SharedPreferences.getInstance();

      await tester.pumpWidget(
        ProviderScope(
          overrides: [
            savedPrefsProvider.overrideWithValue(prefs),
            relayConfigProvider.overrideWith(_FakeRelayConfigNotifier.new),
          ],
          child: const MaterialApp(
            home: SettingsPage(profileHeader: SizedBox.shrink()),
          ),
        ),
      );
      await tester.pump();

      expect(find.text('Notifications'), findsOneWidget);
      expect(find.text('Agent activity'), findsOneWidget);
      expect(find.byType(Switch), findsOneWidget);
    } finally {
      debugDefaultTargetPlatformOverride = previousPlatform;
    }
  });

  testWidgets('does not show the mobile setting on macOS', (tester) async {
    final previousPlatform = debugDefaultTargetPlatformOverride;
    debugDefaultTargetPlatformOverride = TargetPlatform.macOS;
    try {
      SharedPreferences.setMockInitialValues({});
      final prefs = await SharedPreferences.getInstance();

      await tester.pumpWidget(
        ProviderScope(
          overrides: [
            savedPrefsProvider.overrideWithValue(prefs),
            relayConfigProvider.overrideWith(_FakeRelayConfigNotifier.new),
          ],
          child: const MaterialApp(
            home: SettingsPage(profileHeader: SizedBox.shrink()),
          ),
        ),
      );
      await tester.pump();

      expect(find.text('Notifications'), findsNothing);
      expect(find.text('Agent activity'), findsNothing);
      expect(find.byType(Switch), findsNothing);
    } finally {
      debugDefaultTargetPlatformOverride = previousPlatform;
    }
  });
}

class _FakeRelayConfigNotifier extends RelayConfigNotifier {
  @override
  RelayConfig build() => const RelayConfig(baseUrl: 'https://relay.example');
}
