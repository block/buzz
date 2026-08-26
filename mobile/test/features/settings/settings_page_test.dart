import 'package:buzz/features/settings/settings_page.dart';
import 'package:buzz/shared/community/community_membership_provider.dart';
import 'package:buzz/shared/theme/theme.dart';
import 'package:buzz/shared/widgets/app_list.dart';
import 'package:buzz/shared/widgets/app_list_card.dart';
import 'package:flutter/material.dart';
import 'package:flutter/foundation.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';
import 'package:shared_preferences/shared_preferences.dart';

void main() {
  testWidgets('opens profile edit choices and routes photo directly', (
    tester,
  ) async {
    SharedPreferences.setMockInitialValues({});
    final prefs = await SharedPreferences.getInstance();

    await tester.pumpWidget(
      ProviderScope(
        overrides: [savedPrefsProvider.overrideWithValue(prefs)],
        child: MaterialApp(
          theme: AppTheme.light(),
          home: SettingsPage(
            profileHeader: const SizedBox.square(dimension: 128),
            profileEditPageBuilder: (_) =>
                const Scaffold(body: Text('Profile editor destination')),
            invitePageBuilder: (_) => const SizedBox.shrink(),
            identityRecoveryPageBuilder: (_) => const SizedBox.shrink(),
          ),
        ),
      ),
    );
    await tester.pumpAndSettle();

    expect(find.text('Profile'), findsNothing);
    await tester.tap(find.byKey(const ValueKey('settings-edit-profile')));
    await tester.pumpAndSettle();
    expect(find.text('Edit profile'), findsOneWidget);
    expect(find.text('Display name'), findsOneWidget);
    expect(find.text('Profile description'), findsOneWidget);
    expect(find.text('Photo'), findsOneWidget);
    final optionCard = find.descendant(
      of: find.byKey(const ValueKey('edit-profile-options')),
      matching: find.byType(Material),
    );
    expect(tester.getSize(optionCard.first).height, greaterThan(150));
    for (final key in const [
      'edit-profile-display-name',
      'edit-profile-description',
      'edit-profile-photo',
    ]) {
      expect(
        tester.widget<AppListRow>(find.byKey(ValueKey(key))).verticalPadding,
        Grid.xs,
      );
    }
    final sheetContent = tester.widget<Padding>(
      find.byKey(const ValueKey('edit-profile-sheet-content')),
    );
    expect(sheetContent.padding, const EdgeInsets.only(bottom: Grid.xs));
    final sheetSafeArea = tester.widget<SafeArea>(
      find.ancestor(
        of: find.byKey(const ValueKey('edit-profile-sheet-content')),
        matching: find.byType(SafeArea),
      ),
    );
    expect(sheetSafeArea.top, isFalse);
    expect(sheetSafeArea.bottom, isTrue);
    final sheetRect = tester.getRect(find.byType(BottomSheet));
    final optionRect = tester.getRect(
      find.byKey(const ValueKey('edit-profile-options')),
    );
    expect(sheetRect.height, lessThan(340));
    expect(sheetRect.bottom - optionRect.bottom, Grid.xs);
    await tester.tap(find.byKey(const ValueKey('edit-profile-photo')));
    await tester.pumpAndSettle();
    expect(find.text('Profile editor destination'), findsOneWidget);
  });

  testWidgets('opens profile text editors after dismissing the choices', (
    tester,
  ) async {
    SharedPreferences.setMockInitialValues({});
    final prefs = await SharedPreferences.getInstance();
    final opened = <String>[];

    await tester.pumpWidget(
      ProviderScope(
        overrides: [savedPrefsProvider.overrideWithValue(prefs)],
        child: MaterialApp(
          theme: AppTheme.light(),
          home: SettingsPage(
            profileHeader: const SizedBox.shrink(),
            onEditDisplayName: (_) async => opened.add('name'),
            onEditProfileDescription: (_) async => opened.add('description'),
            invitePageBuilder: (_) => const SizedBox.shrink(),
            identityRecoveryPageBuilder: (_) => const SizedBox.shrink(),
          ),
        ),
      ),
    );
    await tester.pumpAndSettle();

    await tester.tap(find.byKey(const ValueKey('settings-edit-profile')));
    await tester.pumpAndSettle();
    await tester.tap(find.byKey(const ValueKey('edit-profile-display-name')));
    await tester.pumpAndSettle();
    expect(opened, ['name']);
    expect(find.text('Edit profile'), findsNothing);

    await tester.tap(find.byKey(const ValueKey('settings-edit-profile')));
    await tester.pumpAndSettle();
    await tester.tap(find.byKey(const ValueKey('edit-profile-description')));
    await tester.pumpAndSettle();
    expect(opened, ['name', 'description']);
  });

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

    final nativeViews = tester.widgetList<UiKitView>(find.byType(UiKitView));
    final nativeClose = nativeViews.singleWhere(
      (view) =>
          (view.creationParams as Map<Object?, Object?>?)?['icon'] == 'close',
    );
    final nativeEdit = nativeViews.singleWhere(
      (view) =>
          (view.creationParams as Map<Object?, Object?>?)?['label'] == 'Edit',
    );
    expect(nativeClose.viewType, 'buzz/navigation_glass');
    expect(nativeClose.creationParams, containsPair('icon', 'close'));
    expect(nativeEdit.viewType, 'buzz/navigation_glass');
    expect(nativeEdit.creationParams, containsPair('controlWidth', 56.0));
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
