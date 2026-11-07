import 'package:buzz/features/settings/settings_page.dart';
import 'package:buzz/shared/agents/owned_agent_directory.dart';
import 'package:buzz/shared/me/default_agent.dart';
import 'package:buzz/shared/theme/theme.dart';
import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';
import 'package:shared_preferences/shared_preferences.dart';

import '../../helpers/widget_helpers.dart';

void main() {
  setUp(() {
    SharedPreferences.setMockInitialValues({});
  });

  testWidgets(
    'renders Agents section in SettingsPage with fallback subtitle when empty',
    (tester) async {
      await tester.pumpWidget(
        WidgetHelpers.testable(
          overrides: [
            defaultAgentProvider.overrideWith((ref) => Stream.value(null)),
            ownedAgentDirectoryProvider.overrideWith((ref) async => const []),
          ],
          child: SettingsPage(
            profileHeader: const SizedBox.shrink(),
            identityRecoveryPageBuilder: (_) => const SizedBox.shrink(),
          ),
        ),
      );
      await tester.pumpAndSettle();

      expect(
        find.byKey(const ValueKey('settings-agents-options')),
        findsOneWidget,
      );
      expect(
        find.byKey(const ValueKey('settings-agents-row')),
        findsOneWidget,
      );
      expect(
        find.text('Manage your Bestie and custom agents'),
        findsOneWidget,
      );
      expect(
        find.byKey(const ValueKey('settings-create-agent-row')),
        findsOneWidget,
      );
      expect(find.text('Create agent'), findsOneWidget);
    },
  );

  testWidgets('triggers onAgents callback when tapping Agents row', (
    tester,
  ) async {
    var onAgentsCalled = false;
    await tester.pumpWidget(
      WidgetHelpers.testable(
        overrides: [
          defaultAgentProvider.overrideWith((ref) => Stream.value(null)),
          ownedAgentDirectoryProvider.overrideWith((ref) async => const []),
        ],
        child: SettingsPage(
          profileHeader: const SizedBox.shrink(),
          identityRecoveryPageBuilder: (_) => const SizedBox.shrink(),
          onAgents: (_) => onAgentsCalled = true,
        ),
      ),
    );
    await tester.pumpAndSettle();

    final agentsRow = find.byKey(const ValueKey('settings-agents-row'));
    expect(agentsRow, findsOneWidget);
    await tester.tap(agentsRow);
    await tester.pump();

    expect(onAgentsCalled, isTrue);
  });

  testWidgets('triggers onCreateAgent callback when tapping Create agent row', (
    tester,
  ) async {
    var onCreateAgentCalled = false;
    await tester.pumpWidget(
      WidgetHelpers.testable(
        overrides: [
          defaultAgentProvider.overrideWith((ref) => Stream.value(null)),
          ownedAgentDirectoryProvider.overrideWith((ref) async => const []),
        ],
        child: SettingsPage(
          profileHeader: const SizedBox.shrink(),
          identityRecoveryPageBuilder: (_) => const SizedBox.shrink(),
          onCreateAgent: (_) => onCreateAgentCalled = true,
        ),
      ),
    );
    await tester.pumpAndSettle();

    final createRow = find.byKey(const ValueKey('settings-create-agent-row'));
    expect(createRow, findsOneWidget);
    await tester.tap(createRow);
    await tester.pump();

    expect(onCreateAgentCalled, isTrue);
  });

  testWidgets('displays agent count and Bestie name in subtitle when configured', (
    tester,
  ) async {
    const bestiePubkey = '953d08f36214227038a8e10ff614ecfa159eb58ac464';
    const otherPubkey = '1234567890abcdef1234567890abcdef1234567890abcdef';

    await tester.pumpWidget(
      WidgetHelpers.testable(
        overrides: [
          defaultAgentProvider.overrideWith(
            (ref) => Stream.value(bestiePubkey),
          ),
          ownedAgentDirectoryProvider.overrideWith(
            (ref) async => const [
              OwnedAgentDirectoryEntry(
                pubkey: bestiePubkey,
                displayName: 'BestieBot',
              ),
              OwnedAgentDirectoryEntry(
                pubkey: otherPubkey,
                displayName: 'HelperBot',
              ),
            ],
          ),
        ],
        child: SettingsPage(
          profileHeader: const SizedBox.shrink(),
          identityRecoveryPageBuilder: (_) => const SizedBox.shrink(),
        ),
      ),
    );
    await tester.pumpAndSettle();

    expect(
      find.byKey(const ValueKey('settings-agents-options')),
      findsOneWidget,
    );
    // Subtitle should reflect the Bestie name and agent count
    expect(find.textContaining('Bestie:'), findsOneWidget);
    expect(find.textContaining('2 agents'), findsOneWidget);
  });

  testWidgets(
    'renders auto-mention agents switch with default false and updates state & prefs',
    (tester) async {
      SharedPreferences.setMockInitialValues({});
      final prefs = await SharedPreferences.getInstance();

      await tester.pumpWidget(
        WidgetHelpers.testable(
          overrides: [
            savedPrefsProvider.overrideWithValue(prefs),
            defaultAgentProvider.overrideWith((ref) => Stream.value(null)),
            ownedAgentDirectoryProvider.overrideWith((ref) async => const []),
          ],
          child: SettingsPage(
            profileHeader: const SizedBox.shrink(),
            identityRecoveryPageBuilder: (_) => const SizedBox.shrink(),
          ),
        ),
      );
      await tester.pumpAndSettle();

      final switchFinder = find.byKey(
        const ValueKey('settings-auto-mention-agents-switch'),
      );
      expect(switchFinder, findsOneWidget);
      expect(tester.widget<Switch>(switchFinder).value, isFalse);

      final rowFinder = find.byKey(
        const ValueKey('settings-auto-mention-agents-row'),
      );
      expect(rowFinder, findsOneWidget);
      expect(find.text('Automatically mention agents'), findsOneWidget);
      expect(
        find.text('Address selected agents in thread replies'),
        findsOneWidget,
      );

      // Tap row to toggle
      await tester.tap(rowFinder);
      await tester.pumpAndSettle();

      expect(tester.widget<Switch>(switchFinder).value, isTrue);
      expect(
        prefs.getBool('buzz.messages.keepMentionedAgentsPinned'),
        isTrue,
      );

      // Tap switch to toggle back
      await tester.tap(switchFinder);
      await tester.pumpAndSettle();

      expect(tester.widget<Switch>(switchFinder).value, isFalse);
      expect(
        prefs.getBool('buzz.messages.keepMentionedAgentsPinned'),
        isFalse,
      );
    },
  );
}
