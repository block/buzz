import 'package:buzz/shared/widgets/mobile_tab_footer_backdrop.dart';
import 'package:buzz/shared/widgets/navigation_backdrop.dart';
import 'package:buzz/shared/widgets/ios_glass_navigation_button.dart';
import 'package:buzz/shared/widgets/adaptive_conversation_pane.dart';

import 'package:buzz/features/activity/activity_page.dart';
import 'package:buzz/shared/profile/user_cache_provider.dart';
import 'package:buzz/shared/profile/user_profile.dart';
import 'package:buzz/features/home/home_page.dart';
import 'package:buzz/features/home/messages_navigation_bar.dart';
import 'package:buzz/shared/widgets/ios_message_create_button.dart';
import 'package:buzz/shared/widgets/ios_navigation_bar.dart';
import 'package:buzz/features/search/search_page.dart';
import 'package:buzz/shared/theme/theme.dart';
import 'package:flutter/foundation.dart';
import 'package:flutter/material.dart';
import 'package:flutter/services.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';

import 'package:buzz/shared/widgets/agent_navigation_overlay.dart';
import 'home_page_harness.dart';

void main() {
  final binding = TestWidgetsFlutterBinding.ensureInitialized();
  setUp(() {
    binding.defaultBinaryMessenger.setMockMethodCallHandler(
      SystemChannels.platform_views,
      (_) async => null,
    );
  });
  tearDown(() {
    binding.defaultBinaryMessenger.setMockMethodCallHandler(
      SystemChannels.platform_views,
      null,
    );
  });
  testWidgets('Duo rail retains unified Messages selection across folds', (
    tester,
  ) async {
    debugDefaultTargetPlatformOverride = TargetPlatform.iOS;
    addTearDown(() => debugDefaultTargetPlatformOverride = null);
    tester.view.devicePixelRatio = 1;
    tester.view.physicalSize = const Size(466, 678);
    tester.view.viewPadding = const FakeViewPadding(right: 76);
    tester.view.padding = const FakeViewPadding(right: 76);
    addTearDown(tester.view.reset);
    final bestie = 'b' * 64;
    await tester.pumpWidget(
      await buildHome(disableAnimations: true, bestie: bestie),
    );
    await tester.pumpAndSettle();
    // The rail replaces the bottom bar and its Flutter glass shortcuts.
    expect(find.byType(MessagesNavigationBar), findsNothing);
    expect(find.byTooltip('Search'), findsNothing);
    // Duo moves the Messages header's Activity and Community menu controls
    // into the rail as native controls, in header order.
    final headerBell = find.byWidgetPredicate(
      (widget) =>
          widget is IosGlassNavigationButton &&
          widget.icon == IosGlassNavigationIcon.bell,
    );
    final headerMenu = find.byWidgetPredicate(
      (widget) =>
          widget is IosMessageCreateButton &&
          widget.accessibilityLabel == 'Community menu',
    );
    final railActivity = find.byKey(const ValueKey('duo-toolbar-Activity'));
    final railMenu = find.byKey(const ValueKey('duo-toolbar-Community menu'));
    IosNavigationAction railAction(Finder control) => tester
        .widget<IosNavigationBar>(
          find.descendant(of: control, matching: find.byType(IosNavigationBar)),
        )
        .leading!;
    expect(headerBell, findsNothing);
    expect(headerMenu, findsNothing);
    expect(tester.getCenter(railActivity).dx, greaterThan(390));
    expect(tester.getCenter(railMenu).dx, tester.getCenter(railActivity).dx);
    expect(
      tester.getCenter(railActivity).dy,
      lessThan(tester.getCenter(railMenu).dy),
    );
    expect(railAction(railActivity).activityColor, isNull);
    expect(
      railAction(railMenu).children.map((action) => action.label),
      containsAllInOrder(['Agents', 'Settings']),
    );
    railAction(railActivity).onPressed!();
    await tester.pumpAndSettle();
    expect(find.byType(ActivityPage), findsOneWidget);
    expect(railActivity, findsNothing);
    Navigator.of(tester.element(find.byType(ActivityPage))).pop();
    await tester.pumpAndSettle();
    expect(railActivity, findsOneWidget);
    // Search, New and Bestie are native rail controls in one capsule pinned to
    // the bottom of the rail: three 48-point controls and the 12-point margin.
    final railBottom = find.byKey(const ValueKey('messages-rail-controls'));
    IosNavigationBar bottomControl() => tester.widget<IosNavigationBar>(
      find.descendant(of: railBottom, matching: find.byType(IosNavigationBar)),
    );
    expect(tester.getSize(railBottom).height, 156);
    // The bottom group shares the rail's center line with the top controls.
    expect(tester.getCenter(railBottom).dx, tester.getCenter(railActivity).dx);
    expect(tester.getBottomLeft(railBottom).dy, 678);
    expect(
      tester.getTopLeft(railBottom).dy,
      greaterThan(tester.getBottomLeft(railActivity).dy),
    );
    expect(bottomControl().standaloneControl, isTrue);
    // While Bestie records, the top rail controls disappear and
    // Search and New go inert, so a second finger cannot open a route that
    // cancels the recording. Bestie keeps its slot and native action: changing
    // the payload would rebuild the capsule and cancel the hold.
    final inert = find.byKey(const ValueKey('messages-rail-inert'));
    void recording(bool value) => AgentNavigationOverlay.recordingCallbackOf(
      tester.element(
        find.descendant(
          of: railBottom,
          matching: find.byType(IosNavigationBar),
        ),
      ),
    )!(value);
    expect(inert, findsNothing);
    recording(true);
    await tester.pumpAndSettle();
    expect(railActivity, findsNothing);
    expect(railMenu, findsNothing);
    expect(railBottom, findsOneWidget);
    expect(inert, findsOneWidget);
    expect(tester.getTopLeft(inert), tester.getTopLeft(railBottom));
    expect(
      tester.getBottomRight(inert).dy,
      tester.getBottomRight(railBottom).dy - 54,
    );
    expect(bottomControl().actions.map((action) => action.label), [
      'Search',
      'New',
      'Chat with your Bestie',
    ]);
    recording(false);
    await tester.pumpAndSettle();
    expect(inert, findsNothing);
    expect(railActivity, findsOneWidget);
    expect(railMenu, findsOneWidget);
    var bottomActions = bottomControl().actions;
    expect(bottomActions.map((action) => action.label), [
      'Search',
      'New',
      'Chat with your Bestie',
    ]);
    // New is a native menu, like the Community menu, not a button.
    expect(bottomActions[1].onPressed, isNull);
    expect(bottomActions[1].children.map((action) => action.label), [
      'New chat',
      'New workspace',
      'New DM',
      'New channel',
    ]);
    // Once the Bestie's profile is known, the same slot shows its avatar. Tap
    // opens the conversation; hold needs an account to record with.
    ProviderScope.containerOf(tester.element(find.byType(HomePage)))
        .read(userCacheProvider.notifier)
        .put(UserProfile(pubkey: bestie, displayName: 'Pip'));
    await tester.pumpAndSettle();
    bottomActions = bottomControl().actions;
    expect(bottomActions[2].label, 'Chat with Pip');
    expect(bottomActions[2].avatarInitial, 'P');
    expect(bottomActions[2].avatarIsAgent, isTrue);
    expect(bottomActions[2].onLongPress, isNull);
    expect(bottomActions[2].onPressed, isNotNull);
    final shortcut = find.descendant(
      of: railBottom,
      matching: find.byType(IconButton),
    );
    expect(tester.getSize(shortcut), const Size.square(48));
    bottomActions.first.onPressed!();
    await tester.pumpAndSettle();
    expect(find.byType(SearchPage), findsOneWidget);
    expect(railBottom, findsNothing);
    Navigator.of(tester.element(find.byType(SearchPage))).pop();
    await tester.pumpAndSettle();
    expect(railBottom, findsOneWidget);
    expect(find.text('Personal workspace'), findsOneWidget);
    final pane = ConversationPaneScope.maybeOf(
      tester.element(find.text('Personal workspace')),
    )!;
    pane.open(
      'sample',
      (_) => const Scaffold(body: Text('Conversation content')),
    );
    await tester.pumpAndSettle();
    expect(find.text('Conversation content'), findsOneWidget);
    expect(find.byType(MessagesNavigationBar), findsNothing);
    expect(railActivity, findsNothing);
    expect(railBottom, findsNothing);
    tester.view.physicalSize = const Size(951, 669);
    await tester.pumpAndSettle();
    // Wide Duo keeps the bottom bar inside the list pane.
    expect(railBottom, findsNothing);
    expect(find.byType(MessagesNavigationBar), findsOneWidget);
    expect(find.text('Conversation content'), findsOneWidget);
    expect(pane.selectedId, 'sample');
    // The list's rail controls stay above the open conversation's.
    expect(railActivity, findsOneWidget);
    expect(railMenu, findsOneWidget);
    final openNavigation = find.byType(MessagesNavigationBar);
    final openBounds = tester.getRect(openNavigation);
    expect(openBounds.left, greaterThanOrEqualTo(0));
    expect(openBounds.right, lessThanOrEqualTo(375));
    expect(openBounds.center.dx, 375 / 2);
    expect(669 - openBounds.bottom, mobileTabBarBottomGap);
    final openCreate = tester.widget<IosMessageCreateButton>(
      find.descendant(
        of: openNavigation,
        matching: find.byType(IosMessageCreateButton),
      ),
    );
    expect(openCreate.navigationAxis, Axis.horizontal);
    openCreate.onMenuVisibilityChanged!(true);
    await tester.pumpAndSettle();
    expect(tester.takeException(), isNull);
    openCreate.onMenuVisibilityChanged!(false);
    await tester.pumpAndSettle();
    tester.widget<MessagesNavigationBar>(openNavigation).onSearch();
    await tester.pumpAndSettle();
    expect(tester.getSize(find.byType(SearchPage)).width, 375);
    tester.view.physicalSize = const Size(466, 678);
    await tester.pumpAndSettle();
    final searchHeader = find.descendant(
      of: find.byType(SearchPage),
      matching: find.byType(NavigationBackdrop),
    );
    final searchField = find.byKey(const Key('search-field-container'));
    expect(tester.getRect(searchHeader).left, 0);
    expect(tester.getRect(searchHeader).right, 466);
    expect(tester.getRect(searchField).top, Grid.xs);
    expect(tester.getRect(searchField).right, lessThanOrEqualTo(390));
    tester.view.physicalSize = const Size(951, 669);
    await tester.pumpAndSettle();
    expect(tester.getSize(find.byType(SearchPage)).width, 375);
    expect(tester.getRect(searchField).top, Grid.twelve);
    Navigator.of(tester.element(find.byType(SearchPage))).pop();
    await tester.pumpAndSettle();
    tester.view.physicalSize = const Size(669, 951);
    tester.view.viewPadding = const FakeViewPadding(top: 82, bottom: 34);
    tester.view.padding = const FakeViewPadding(top: 82, bottom: 34);
    await tester.pumpAndSettle();
    expect(find.byType(MessagesNavigationBar), findsNothing);
    expect(find.text('Personal workspace'), findsNothing);
    expect(find.text('Conversation content'), findsOneWidget);
    expect(pane.selectedId, 'sample');
    tester.view.viewPadding = const FakeViewPadding(right: 76);
    tester.view.padding = const FakeViewPadding(right: 76);
    tester.view.physicalSize = const Size(951, 669);
    await tester.pumpAndSettle();
    expect(find.text('Personal workspace'), findsOneWidget);
    expect(find.text('Conversation content'), findsOneWidget);
    tester.view.physicalSize = const Size(466, 678);
    await tester.pumpAndSettle();
    await tester.binding.handlePopRoute();
    await tester.pumpAndSettle();
    expect(find.text('Personal workspace'), findsOneWidget);
    expect(railBottom, findsOneWidget);
    expect(find.byType(MessagesNavigationBar), findsNothing);
    tester.view.physicalSize = const Size(390, 844);
    tester.view.viewPadding = const FakeViewPadding(bottom: 34);
    tester.view.padding = const FakeViewPadding(bottom: 34);
    await tester.pumpAndSettle();
    // An ordinary iPhone keeps its bottom bar and the header controls.
    expect(find.byType(MessagesNavigationBar), findsOneWidget);
    expect(railBottom, findsNothing);
    expect(headerBell, findsOneWidget);
    expect(headerMenu, findsOneWidget);
    expect(railActivity, findsNothing);
    expect(tester.takeException(), isNull);
    debugDefaultTargetPlatformOverride = null;
  });
}
