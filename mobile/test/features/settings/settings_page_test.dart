import 'package:buzz/features/settings/settings_page.dart';
import 'package:buzz/shared/community/community_membership_provider.dart';
import 'package:buzz/shared/theme/theme.dart';
import 'package:buzz/shared/widgets/app_list_card.dart';
import 'package:flutter/material.dart';
import 'package:flutter/foundation.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';
import 'package:shared_preferences/shared_preferences.dart';

void main() {
  testWidgets('uses the native glass close control on iOS', (tester) async {
    debugDefaultTargetPlatformOverride = TargetPlatform.iOS;
    addTearDown(() => debugDefaultTargetPlatformOverride = null);
    SharedPreferences.setMockInitialValues({});
    final prefs = await SharedPreferences.getInstance();

    await tester.pumpWidget(
      ProviderScope(
        overrides: [savedPrefsProvider.overrideWithValue(prefs)],
        child: MaterialApp(
          theme: AppTheme.light(),
          home: SettingsPage(
            profileHeader: const SizedBox.shrink(),
            invitePageBuilder: (_) => const SizedBox.shrink(),
            identityRecoveryPageBuilder: (_) => const SizedBox.shrink(),
          ),
        ),
      ),
    );
    await tester.pumpAndSettle();

    final nativeClose = tester.widget<UiKitView>(find.byType(UiKitView));
    expect(nativeClose.viewType, 'buzz/navigation_glass');
    expect(nativeClose.creationParams, containsPair('icon', 'close'));
    expect(find.byTooltip('Close settings'), findsOneWidget);
    debugDefaultTargetPlatformOverride = null;
  });

  testWidgets('shows community invite navigation to owners and admins', (
    tester,
  ) async {
    SharedPreferences.setMockInitialValues({});
    final prefs = await SharedPreferences.getInstance();

    await tester.pumpWidget(
      ProviderScope(
        overrides: [
          savedPrefsProvider.overrideWithValue(prefs),
          currentCommunityRoleProvider.overrideWithValue(
            const AsyncData<CommunityMemberRole?>(CommunityMemberRole.admin),
          ),
        ],
        child: MaterialApp(
          theme: AppTheme.light(),
          home: SettingsPage(
            profileHeader: const SizedBox.shrink(),
            invitePageBuilder: (_) => const Text('Invite destination'),
            identityRecoveryPageBuilder: (_) => const SizedBox.shrink(),
          ),
        ),
      ),
    );
    await tester.pumpAndSettle();

    expect(find.text('Invite to community'), findsOneWidget);
    expect(
      find.text('Add people directly or share an invite link'),
      findsNothing,
    );
    await tester.tap(find.text('Invite to community'));
    await tester.pumpAndSettle();
    expect(find.text('Invite destination'), findsOneWidget);
  });

  testWidgets('keeps invite navigation available when role lookup fails', (
    tester,
  ) async {
    SharedPreferences.setMockInitialValues({});
    final prefs = await SharedPreferences.getInstance();

    await tester.pumpWidget(
      ProviderScope(
        overrides: [
          savedPrefsProvider.overrideWithValue(prefs),
          currentCommunityRoleProvider.overrideWithValue(
            AsyncError<CommunityMemberRole?>(
              Exception('membership query failed'),
              StackTrace.empty,
            ),
          ),
        ],
        child: MaterialApp(
          theme: AppTheme.light(),
          home: SettingsPage(
            profileHeader: const SizedBox.shrink(),
            invitePageBuilder: (_) => const Text('Invite destination'),
            identityRecoveryPageBuilder: (_) => const SizedBox.shrink(),
          ),
        ),
      ),
    );
    await tester.pumpAndSettle();

    expect(find.text('Invite to community'), findsOneWidget);
    await tester.tap(find.text('Invite to community'));
    await tester.pumpAndSettle();
    expect(find.text('Invite destination'), findsOneWidget);
  });

  testWidgets('hides community invite navigation from plain members', (
    tester,
  ) async {
    SharedPreferences.setMockInitialValues({});
    final prefs = await SharedPreferences.getInstance();

    await tester.pumpWidget(
      ProviderScope(
        overrides: [
          savedPrefsProvider.overrideWithValue(prefs),
          currentCommunityRoleProvider.overrideWithValue(
            const AsyncData<CommunityMemberRole?>(CommunityMemberRole.member),
          ),
        ],
        child: MaterialApp(
          theme: AppTheme.light(),
          home: SettingsPage(
            profileHeader: const SizedBox.shrink(),
            invitePageBuilder: (_) => const Text('Invite destination'),
            identityRecoveryPageBuilder: (_) => const SizedBox.shrink(),
          ),
        ),
      ),
    );
    await tester.pumpAndSettle();

    expect(find.text('Invite to community'), findsNothing);
  });

  testWidgets('uses a 24dp rhythm between profile settings groups', (
    tester,
  ) async {
    tester.view.physicalSize = const Size(390, 1600);
    tester.view.devicePixelRatio = 1;
    addTearDown(tester.view.reset);
    SharedPreferences.setMockInitialValues({});
    final prefs = await SharedPreferences.getInstance();

    await tester.pumpWidget(
      ProviderScope(
        overrides: [
          savedPrefsProvider.overrideWithValue(prefs),
          currentCommunityRoleProvider.overrideWithValue(
            const AsyncData<CommunityMemberRole?>(CommunityMemberRole.admin),
          ),
        ],
        child: MaterialApp(
          theme: AppTheme.light(),
          home: SettingsPage(
            profileHeader: const SizedBox(height: 100),
            invitePageBuilder: (_) => const SizedBox.shrink(),
            identityRecoveryPageBuilder: (_) => const SizedBox.shrink(),
          ),
        ),
      ),
    );
    await tester.pumpAndSettle();

    final sections = tester.widgetList<AppListCard>(find.byType(AppListCard));
    expect(sections, isNotEmpty);
    expect(
      sections.every((section) => section.verticalPadding == Grid.twelve),
      isTrue,
    );
  });
}
