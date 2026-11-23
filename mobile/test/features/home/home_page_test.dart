import 'package:buzz/shared/widgets/mobile_tab_footer_backdrop.dart';
import 'package:buzz/shared/widgets/navigation_backdrop.dart';
import 'package:buzz/shared/widgets/ios_glass_navigation_button.dart';
import 'package:buzz/shared/widgets/adaptive_conversation_pane.dart';

import 'dart:async';

import 'package:buzz/features/channels/default_agent_launcher.dart';
import 'package:buzz/features/agents/agents_page.dart';
import 'package:buzz/features/channels/me/me_section.dart';
import 'package:buzz/features/activity/activity_page.dart';
import 'package:buzz/features/activity/inbox_unread_provider.dart';
import 'package:buzz/features/channels/channel.dart';
import 'package:buzz/features/channels/channel_detail_page.dart';
import 'package:buzz/features/profile/profile_edit_page.dart';
import 'package:buzz/features/channels/channels_page.dart';
import 'package:buzz/features/channels/channels_provider.dart';
import 'package:buzz/shared/me/default_agent.dart';
import 'package:buzz/shared/relay/app_lifecycle_provider.dart';
import 'package:buzz/shared/profile/user_cache_provider.dart';
import 'package:buzz/shared/profile/user_profile.dart';
import 'package:buzz/shared/shortcuts/home_screen_shortcuts.dart';
import 'package:buzz/shared/widgets/frosted_app_bar.dart';
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
import 'package:shared_preferences/shared_preferences.dart';

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
  Future<Widget> buildHome({
    bool longSolo = false,
    bool inboxUnread = false,
    bool disableAnimations = false,
    Gradient? topSectionGradient,
    ValueChanged<ValueNotifier<VoidCallback?>>? onNewChatAction,
    String? bestie,
    Stream<String?>? bestieUpdates,
    ValueChanged<String>? onOpenBestie,
  }) async {
    SharedPreferences.setMockInitialValues({});
    final prefs = await SharedPreferences.getInstance();
    return ProviderScope(
      overrides: [
        savedPrefsProvider.overrideWithValue(prefs),
        inboxHasUnreadProvider.overrideWithValue(inboxUnread),
        defaultAgentProvider.overrideWith(
          (ref) => bestieUpdates ?? Stream.value(bestie),
        ),
        channelsProvider.overrideWith(_Channels.new),
        // Lifecycle changes here exercise Home, not the relay connection.
        appLifecycleProvider.overrideWith(_Lifecycle.new),
      ],
      child: MaterialApp(
        theme: AppTheme.light(topSectionGradient: topSectionGradient),
        builder: (context, child) => MediaQuery(
          data: MediaQuery.of(
            context,
          ).copyWith(disableAnimations: disableAnimations),
          child: child!,
        ),
        home: HomePage(
          onAgents: showAgents,
          openBestieConversation: (_, _, pubkey) async =>
              onOpenBestie?.call(pubkey),
          defaultAgentBuilder: (context) =>
              DefaultAgentLauncher(onChooseAgent: () => showAgents(context)),
          mePageBuilder: (_, action, workspace) {
            onNewChatAction?.call(action);
            return SliverToBoxAdapter(
              child: MeSection(
                label: 'Chats',
                children: [
                  const Text('Personal workspace'),
                  if (longSolo) ...[
                    const SizedBox(height: 1200),
                    const SizedBox(height: 44, child: Text('Last solo chat')),
                  ],
                ],
              ),
            );
          },
          settingsPageBuilder: _buildSettingsPage,
        ),
      ),
    );
  }

  testWidgets('every Home shortcut inherits the conversation controller', (
    tester,
  ) async {
    tester.view.devicePixelRatio = 1;
    addTearDown(tester.view.reset);
    for (final scenario in [
      (size: const Size(1024, 768), rail: 0.0),
      (size: const Size(466, 678), rail: 76.0),
      (size: const Size(951, 669), rail: 76.0),
    ]) {
      tester.view.physicalSize = scenario.size;
      tester.view.viewPadding = FakeViewPadding(right: scenario.rail);
      tester.view.padding = tester.view.viewPadding;
      await tester.pumpWidget(await buildHome());
      await tester.pumpAndSettle();
      final launcherContext = tester.element(find.byType(DefaultAgentLauncher));
      final controller = tester
          .widget<AdaptiveConversationPane>(
            find.byType(AdaptiveConversationPane),
          )
          .controller;
      expect(ConversationPaneScope.maybeOf(launcherContext), same(controller));
      await tester.pumpWidget(const SizedBox.shrink());
    }
  });

  testWidgets('keyboard in a phone conversation does not rebuild Home', (
    tester,
  ) async {
    tester.view.physicalSize = const Size(402, 874);
    tester.view.devicePixelRatio = 1;
    addTearDown(tester.view.reset);
    var homeBuilds = 0;
    await tester.pumpWidget(
      await buildHome(onNewChatAction: (_) => homeBuilds++),
    );
    await tester.pumpAndSettle();
    tester
        .widget<AdaptiveConversationPane>(find.byType(AdaptiveConversationPane))
        .controller
        .open(
          'sample',
          (_) => const Scaffold(body: Text('Conversation content')),
        );
    await tester.pumpAndSettle();
    expect(find.text('Conversation content'), findsOneWidget);
    homeBuilds = 0;

    for (var frame = 1; frame <= 20; frame++) {
      tester.view.viewInsets = FakeViewPadding(bottom: frame * 15.0);
      await tester.pump(const Duration(milliseconds: 16));
    }

    expect(homeBuilds, 0);
  });

  for (final (name, size, rightInset, platform) in [
    ('phone', const Size(402, 874), 0.0, null),
    ('iOS Duo', const Size(951, 669), 76.0, TargetPlatform.iOS),
  ]) {
    testWidgets('a sheet opening above Home does not rebuild its inventory '
        '($name)', (tester) async {
      debugDefaultTargetPlatformOverride = platform;
      addTearDown(() => debugDefaultTargetPlatformOverride = null);
      tester.view.physicalSize = size;
      tester.view.viewPadding = FakeViewPadding(right: rightInset);
      tester.view.padding = tester.view.viewPadding;
      tester.view.devicePixelRatio = 1;
      addTearDown(tester.view.reset);
      await tester.pumpWidget(await buildHome());
      await tester.pumpAndSettle();
      final rebuilds = <String>[];
      debugOnRebuildDirtyWidget = (element, _) {
        final type = '${element.widget.runtimeType}';
        if (const {'HomePage', 'ChannelsPage', '_ChannelTile'}.contains(type)) {
          rebuilds.add(type);
        }
      };
      addTearDown(() => debugOnRebuildDirtyWidget = null);

      unawaited(
        showModalBottomSheet<void>(
          context: tester.element(find.byType(ChannelsPage)),
          useRootNavigator: true,
          builder: (_) => const Text('Sheet'),
        ),
      );
      await tester.pumpAndSettle();
      expect(find.text('Sheet'), findsOneWidget);
      Navigator.of(tester.element(find.text('Sheet'))).pop();
      await tester.pumpAndSettle();
      debugOnRebuildDirtyWidget = null;
      debugDefaultTargetPlatformOverride = null;

      expect(rebuilds, isEmpty);
    });
  }

  testWidgets('feedback clears floating navigation and wraps large text', (
    tester,
  ) async {
    tester.view.physicalSize = const Size(390, 844);
    tester.view.devicePixelRatio = 1;
    tester.view.viewPadding = const FakeViewPadding(bottom: 34);
    addTearDown(tester.view.reset);
    await tester.pumpWidget(await buildHome());
    await tester.pumpAndSettle();
    final navigation = find.byType(MessagesNavigationBar);
    final originalTop = tester.getTopLeft(navigation).dy;
    final context = tester.element(find.text('General'));
    final messenger = ScaffoldMessenger.of(context);
    messenger.showSnackBar(const SnackBar(content: Text('Copied')));
    await tester.pumpAndSettle();
    expect(
      tester.getBottomLeft(find.byType(SnackBar)).dy,
      lessThanOrEqualTo(originalTop),
    );
    expect(tester.getTopLeft(navigation).dy, originalTop);
    messenger.removeCurrentSnackBar();
    messenger.showSnackBar(
      const SnackBar(
        content: Text(
          'Could not share this image. Please try again in a moment.',
          textScaler: TextScaler.linear(2),
        ),
      ),
    );
    await tester.pumpAndSettle();
    expect(
      tester.getBottomLeft(find.byType(SnackBar)).dy,
      lessThanOrEqualTo(tester.getTopLeft(navigation).dy),
    );
    expect(tester.takeException(), isNull);
    messenger.removeCurrentSnackBar();
    await tester.pumpAndSettle();
  });

  testWidgets(
    'Duo Messages keeps its title and switcher aligned across folds',
    (tester) async {
      tester.view.devicePixelRatio = 1;
      tester.view.physicalSize = const Size(951, 669);
      tester.view.viewPadding = const FakeViewPadding(right: 76);
      tester.view.padding = const FakeViewPadding(right: 76);
      addTearDown(tester.view.reset);
      await tester.pumpWidget(await buildHome(longSolo: true));
      await tester.pumpAndSettle();
      final bar = find.byWidgetPredicate(
        (widget) => widget is FrostedAppBar && widget.nativeTitle == 'Messages',
      );
      expect(tester.widget<FrostedAppBar>(bar).nativeLargeTitle, isTrue);
      expect(tester.getSize(bar).width, 375);
      expect(
        find.byWidgetPredicate((w) => w is Image && w.semanticLabel == 'Buzz'),
        findsOneWidget,
      );
      expect(tester.getTopLeft(bar).dy, 0);
      final menu = find.byWidgetPredicate(
        (widget) =>
            widget is IosMessageCreateButton &&
            widget.accessibilityLabel == 'Community menu',
      );
      final nativeTitle = find.byWidgetPredicate(
        (w) => w is IosNavigationBar && w.title == 'Messages',
      );
      final navigation = tester.widget<IosNavigationBar>(nativeTitle);
      expect(navigation.alignCollapsedTitleToLeading, isFalse);
      final options = find.byTooltip('Messages options');
      final optionsAtRest = tester.getCenter(options);
      expect(optionsAtRest.dx, lessThan(tester.getCenter(bar).dx));
      expect(navigation.prefersSoftTopEdge, isTrue);
      final controlsAtRest = tester.getCenter(menu);
      final originalHeaderRect = tester.getRect(bar);
      final scrollable = find.byType(Scrollable).first;
      await tester.drag(scrollable, const Offset(0, -250));
      await tester.pumpAndSettle();
      expect(tester.getSize(bar).height, lessThan(originalHeaderRect.height));
      expect(tester.getCenter(menu), controlsAtRest);
      expect(tester.getCenter(options), optionsAtRest);
      expect(
        find.byWidgetPredicate(
          (widget) =>
              widget is IosMessageCreateButton &&
              widget.accessibilityLabel == 'Community menu',
        ),
        findsOneWidget,
      );
      tester.view.physicalSize = const Size(466, 678);
      await tester.pumpAndSettle();
      expect(tester.widget<FrostedAppBar>(bar).nativeLargeTitle, isTrue);
      expect(
        tester
            .widget<IosNavigationBar>(nativeTitle)
            .alignCollapsedTitleToLeading,
        isTrue,
      );
      expect(tester.getSize(find.byTooltip('Search')), const Size.square(48));
      final coverMenu = tester.widget<IosMessageCreateButton>(
        find.byKey(const ValueKey('vertical-create-menu')),
      );
      expect(coverMenu.width, 48);
      final closedHeaderRect = tester.getRect(bar);
      expect(closedHeaderRect.left, 0);
      expect(closedHeaderRect.right, 466);
      expect(tester.getRect(scrollable).right, lessThanOrEqualTo(390));
      expect(tester.getRect(menu).right, lessThanOrEqualTo(466 - 76));
      final bell = find.byWidgetPredicate(
        (w) => w is IosGlassNavigationButton && w.semanticLabel == 'Activity',
      );
      expect(tester.getCenter(menu).dy, closeTo(47.8, .01));
      expect(tester.getCenter(bell).dy, closeTo(47.8, .01));
      final railOptions = find.byKey(
        const ValueKey('duo-toolbar-Messages options'),
      );
      expect(
        tester.getCenter(railOptions).dx,
        closeTo(tester.getCenter(find.byTooltip('Search')).dx, 1),
      );
      expect(options, findsNothing);
      expect(
        tester.widget<FrostedAppBar>(bar).alignLargeTitleWithActions,
        isTrue,
      );

      await tester.drag(scrollable, const Offset(0, -200));
      await tester.pumpAndSettle();
      expect(tester.getRect(bar).height, lessThan(closedHeaderRect.height));
      tester.view.physicalSize = const Size(951, 669);
      tester.view.viewPadding = const FakeViewPadding(left: 76);
      tester.view.padding = const FakeViewPadding(left: 76);
      await tester.pumpAndSettle();
      expect(tester.getRect(bar).left, 0);
      expect(tester.getSize(bar).width, 375 + 76);
      expect(tester.getRect(nativeTitle).left, greaterThanOrEqualTo(76));
      expect(tester.takeException(), isNull);
    },
    variant: TargetPlatformVariant.only(TargetPlatform.iOS),
  );

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
    await tester.pumpWidget(await buildHome(disableAnimations: true));
    await tester.pumpAndSettle();
    expect(
      tester
          .widget<MessagesNavigationBar>(find.byType(MessagesNavigationBar))
          .axis,
      Axis.vertical,
    );
    expect(tester.getCenter(find.byTooltip('Search')).dx, greaterThan(390));
    final create = tester.widget<IosMessageCreateButton>(
      find.descendant(
        of: find.byType(MessagesNavigationBar),
        matching: find.byType(IosMessageCreateButton),
      ),
    );
    expect(create.mergingNavigation, isTrue);
    expect(create.navigationAxis, Axis.vertical);
    create.onMenuVisibilityChanged!(true);
    await tester.pumpAndSettle();
    expect(
      tester
          .widgetList<IgnorePointer>(
            find.ancestor(
              of: find.byTooltip('Search'),
              matching: find.byType(IgnorePointer),
            ),
          )
          .any((widget) => widget.ignoring),
      isTrue,
    );
    create.onMenuVisibilityChanged!(false);
    await tester.pumpAndSettle();
    expect(
      tester
          .widgetList<IgnorePointer>(
            find.ancestor(
              of: find.byTooltip('Search'),
              matching: find.byType(IgnorePointer),
            ),
          )
          .any((widget) => widget.ignoring),
      isFalse,
    );

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
    tester.view.physicalSize = const Size(951, 669);
    await tester.pumpAndSettle();
    expect(find.byType(MessagesNavigationBar), findsOneWidget);
    expect(find.text('Conversation content'), findsOneWidget);
    expect(pane.selectedId, 'sample');
    final openNavigation = find.byType(MessagesNavigationBar);
    expect(
      tester.widget<MessagesNavigationBar>(openNavigation).axis,
      Axis.horizontal,
    );
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
    expect(find.byType(MessagesNavigationBar), findsOneWidget);
    tester.view.physicalSize = const Size(390, 844);
    tester.view.viewPadding = const FakeViewPadding(bottom: 34);
    tester.view.padding = const FakeViewPadding(bottom: 34);
    await tester.pumpAndSettle();
    expect(
      tester
          .widget<MessagesNavigationBar>(find.byType(MessagesNavigationBar))
          .axis,
      Axis.horizontal,
    );
    expect(tester.takeException(), isNull);
    debugDefaultTargetPlatformOverride = null;
  });

  testWidgets(
    'Messages lists its inventory and opens Search and Settings above it',
    (tester) async {
      await tester.pumpWidget(await buildHome());
      await tester.pumpAndSettle();
      final navigation = find.byType(MessagesNavigationBar);
      final mountedNavigation = tester.element(navigation);
      final focus = find.byKey(const ValueKey('voice-recording-focus'));
      expect(tester.widget<ImageFiltered>(focus).enabled, isFalse);
      tester.widget<MessagesNavigationBar>(navigation).onRecordingChanged!(
        true,
      );
      await tester.pump();
      expect(tester.widget<ImageFiltered>(focus).enabled, isTrue);
      expect(tester.element(navigation), same(mountedNavigation));
      tester.widget<MessagesNavigationBar>(navigation).onRecordingChanged!(
        false,
      );
      await tester.pump();
      expect(tester.widget<ImageFiltered>(focus).enabled, isFalse);
      expect(find.byTooltip('Messages'), findsNothing);
      expect(find.byTooltip('Solo'), findsNothing);
      expect(find.text('Personal workspace'), findsOneWidget);
      expect(find.text('General'), findsOneWidget);
      final header = tester.widget<FrostedAppBar>(find.byType(FrostedAppBar));
      expect(header.useNativeNavigation, isFalse);
      expect(header.nativeLargeTitle, isFalse);
      expect(header.alwaysFrosted, isTrue);
      expect(header.titleStyle?.fontSize, 17);
      expect(header.centerTitle, isTrue);

      final teamSection = find.descendant(
        of: find.byType(CustomScrollView),
        matching: find.text('Team'),
      );
      await tester.tap(find.text('Chats'));
      await tester.tap(teamSection);
      await tester.pumpAndSettle();
      expect(find.text('Personal workspace').hitTestable(), findsNothing);
      expect(find.text('General').hitTestable(), findsNothing);
      await tester.tap(find.text('Chats'));
      await tester.tap(teamSection);
      await tester.pumpAndSettle();
      expect(find.text('Personal workspace').hitTestable(), findsOneWidget);
      expect(find.text('General').hitTestable(), findsOneWidget);
      expect(find.text('Activity'), findsNothing);
      await tester.tap(find.byTooltip('Activity'));
      await tester.pumpAndSettle();
      expect(find.byType(ActivityPage), findsOneWidget);
      Navigator.of(tester.element(find.byType(ActivityPage))).pop();
      await tester.pumpAndSettle();
      await tester.tap(find.byTooltip('Search'));
      await tester.pumpAndSettle();
      expect(find.byType(SearchPage), findsOneWidget);
      expect(find.byKey(const Key('search-close')), findsOneWidget);
      await tester.tap(find.byKey(const Key('search-close')));
      await tester.pumpAndSettle();
      expect(find.text('General'), findsOneWidget);
      expect(find.byType(MessagesNavigationBar), findsOneWidget);
      await tester.tap(find.byTooltip('Community menu'));
      await tester.pumpAndSettle();
      expect(find.text('Switch community'), findsOneWidget);
      expect(find.text('Agents'), findsOneWidget);
      expect(find.text('Settings'), findsOneWidget);
      await tester.tap(find.text('Settings'));
      await tester.pumpAndSettle();
      expect(find.text('Your profile destination'), findsOneWidget);
      Navigator.of(tester.element(find.text('Your profile destination'))).pop();
      await tester.pumpAndSettle();
      expect(find.text('General'), findsOneWidget);
      await tester.pumpWidget(await buildHome(longSolo: true));
      await tester.pumpAndSettle();
      await tester.drag(
        find.byType(CustomScrollView).first,
        const Offset(0, -2000),
      );
      await tester.pumpAndSettle();
      expect(
        tester.getBottomLeft(find.text('Last solo chat')).dy,
        lessThanOrEqualTo(
          tester.getTopLeft(find.byType(MessagesNavigationBar)).dy,
        ),
      );
    },
  );

  for (final reduced in [false, true]) {
    testWidgets('New opens creation actions with reduced motion $reduced', (
      tester,
    ) async {
      ValueNotifier<VoidCallback?>? chat;
      var created = 0;
      await tester.pumpWidget(
        await buildHome(
          disableAnimations: reduced,
          onNewChatAction: (value) => chat = value,
        ),
      );
      await tester.pumpAndSettle();
      chat!.value = () => created++;
      await tester.pumpAndSettle();
      final controlsBounds = tester.getRect(find.byType(MessagesNavigationBar));
      await tester.tap(find.byTooltip('New'));
      await tester.pumpAndSettle();
      expect(find.text('New workspace'), findsOneWidget);
      final menu = find
          .ancestor(
            of: find.text('New workspace'),
            matching: find.byType(Material),
          )
          .first;
      final menuBounds = tester.getRect(menu);
      expect(menuBounds.left, closeTo(controlsBounds.left, .01));
      expect(menuBounds.right, closeTo(controlsBounds.right, .01));
      expect(menuBounds.bottom, closeTo(controlsBounds.bottom, .01));
      final rows = ['New chat', 'New channel', 'New DM', 'New workspace'];
      final positions = rows.map(
        (label) => tester.getCenter(find.text(label)).dy,
      );
      expect(positions, orderedEquals(positions.toList()..sort()));
      final divider = find.descendant(of: menu, matching: find.byType(Divider));
      expect(
        tester.getCenter(divider).dy,
        inExclusiveRange(positions.elementAt(2), positions.last),
      );
      for (final heading in ['Solo', 'Team']) {
        expect(
          find.descendant(of: menu, matching: find.text(heading)),
          findsNothing,
        );
      }
      await tester.tap(find.text('New chat'));
      await tester.pumpAndSettle();
      expect(created, 1);
    });
  }

  testWidgets('app icon shortcuts name the Bestie and open each destination', (
    tester,
  ) async {
    tester.view.devicePixelRatio = 1;
    tester.view.physicalSize = const Size(402, 874);
    addTearDown(tester.view.reset);
    addTearDown(() => pendingHomeScreenShortcut.value = null);
    const channel = MethodChannel('buzz/home_screen_shortcuts');
    final published = <List<String>>[];
    tester.binding.defaultBinaryMessenger.setMockMethodCallHandler(channel, (
      call,
    ) async {
      if (call.method == 'setItems') {
        published.add([
          for (final item in call.arguments as List)
            '${(item as Map)['title']}',
        ]);
      }
      return null;
    });
    addTearDown(
      () => tester.binding.defaultBinaryMessenger.setMockMethodCallHandler(
        channel,
        null,
      ),
    );
    Future<Object?> tap(String type) async {
      Object? reply;
      await tester.binding.defaultBinaryMessenger.handlePlatformMessage(
        channel.name,
        const StandardMethodCodec().encodeMethodCall(MethodCall('open', type)),
        (data) => reply = const StandardMethodCodec().decodeEnvelope(data!),
      );
      await tester.pumpAndSettle();
      return reply;
    }

    final bestie = 'b' * 64;
    final bestieUpdates = StreamController<String?>();
    addTearDown(() async {
      await tester.pumpWidget(const SizedBox());
      await bestieUpdates.close();
    });
    bestieUpdates.add(bestie);
    ValueNotifier<VoidCallback?>? chat;
    var homeBuilds = 0;
    var created = 0;
    final opened = <String>[];
    await tester.pumpWidget(
      await buildHome(
        bestieUpdates: bestieUpdates.stream,
        onOpenBestie: opened.add,
        onNewChatAction: (value) {
          chat = value;
          homeBuilds++;
        },
      ),
    );
    await tester.pumpAndSettle();
    tester.binding.handleAppLifecycleStateChanged(AppLifecycleState.inactive);
    // A tap that launched the app waits for its destination to be ready.
    expect(await tap('new-chat'), 'handled');
    expect(created, 0);
    chat!.value = () => created++;
    ProviderScope.containerOf(tester.element(find.byType(HomePage)))
        .read(userCacheProvider.notifier)
        .put(UserProfile(pubkey: bestie, displayName: 'Pip'));
    await tester.pumpAndSettle();
    // iOS delivers the tap before the app is active again.
    expect(created, 0);
    tester.binding.handleAppLifecycleStateChanged(AppLifecycleState.resumed);
    await tester.pumpAndSettle();
    expect(created, 1);
    expect(published.last, ['New chat', 'New DM', 'Message Pip']);

    // A tap with another screen open returns to Home first.
    Navigator.of(
      tester.element(find.byType(HomePage)),
    ).push(MaterialPageRoute<void>(builder: (_) => const Text('Open sheet')));
    await tester.pumpAndSettle();
    await tap('message-bestie');
    expect(find.text('Open sheet'), findsNothing);
    expect(opened, [bestie]);
    await tap('new-chat');
    expect(created, 2);

    // With a New chat draft open, the tap leaves the draft and starts one.
    final startChat = chat!.value;
    Navigator.of(
      tester.element(find.byType(HomePage)),
    ).push(MaterialPageRoute<void>(builder: (_) => const Text('Draft')));
    chat!.value = null;
    await tester.pumpAndSettle();
    await tap('new-chat');
    expect(find.text('Draft'), findsNothing);
    expect(created, 2);
    chat!.value = startChat;
    await tester.pumpAndSettle();
    expect(created, 3);
    expect(pendingHomeScreenShortcut.value, isNull);

    // A New DM tap with a New DM picker already open replaces it, and the
    // replacement keeps receiving the native picker's callbacks.
    const dmBridge = MethodChannel('buzz/add_members_sheet');
    final dmStates = <Map<Object?, Object?>>[];
    final dmCalls = <String>[];
    String? nativeSession;
    tester.binding.defaultBinaryMessenger.setMockMethodCallHandler(dmBridge, (
      call,
    ) async {
      final args = Map<Object?, Object?>.from(call.arguments as Map);
      dmCalls.add('${call.method}:${args['session']}');
      if (call.method == 'present') {
        if (nativeSession != null) throw PlatformException(code: 'busy');
        nativeSession = args['session'] as String;
        dmStates.add(args);
      } else if (call.method == 'dismiss') {
        if (nativeSession == args['session']) nativeSession = null;
      } else if (call.method == 'update') {
        dmStates.add(args);
      }
      return null;
    });
    addTearDown(
      () => tester.binding.defaultBinaryMessenger.setMockMethodCallHandler(
        dmBridge,
        null,
      ),
    );
    await tap('new-dm');
    expect(dmStates.last['title'], 'New DM');
    final firstSession = nativeSession;
    expect(firstSession, isNotNull);
    await tap('new-dm');
    expect(nativeSession, isNotNull, reason: dmCalls.join('\n'));
    expect(nativeSession, isNot(firstSession));
    await tester.binding.defaultBinaryMessenger.handlePlatformMessage(
      dmBridge.name,
      const StandardMethodCodec().encodeMethodCall(
        MethodCall('closed', {'session': nativeSession}),
      ),
      (_) {},
    );
    await tester.pumpAndSettle();
    expect(
      Navigator.of(tester.element(find.byType(HomePage))).canPop(),
      isFalse,
      reason: dmCalls.join(', '),
    );

    // Search uses a pane even on an ordinary phone. Drive the real search
    // result and channel page, then dispatch each shortcut through the bridge.
    Future<void> openSearchResult() async {
      tester
          .widget<MessagesNavigationBar>(find.byType(MessagesNavigationBar))
          .onSearch();
      await tester.pumpAndSettle();
      await tester.enterText(find.byType(TextField), '#');
      // Advance the production search debounce, then settle its results.
      await tester.pump(const Duration(milliseconds: 300));
      await tester.pumpAndSettle();
      await tester.tap(
        find.byKey(const ValueKey('search-channel-row-channel')),
      );
      await tester.pumpAndSettle();
      expect(find.byType(ChannelDetailPage), findsOneWidget);
    }

    for (final shortcut in HomeScreenShortcut.values) {
      await openSearchResult();
      final beforeChat = created;
      final beforeBestie = opened.length;
      final beforeDm = dmStates.length;
      await tap(shortcut.type);
      expect(find.byType(ChannelDetailPage), findsNothing);
      expect(find.byType(SearchPage), findsNothing);
      expect(pendingHomeScreenShortcut.value, isNull);
      switch (shortcut) {
        case HomeScreenShortcut.newChat:
          expect(created, beforeChat + 1);
        case HomeScreenShortcut.messageBestie:
          expect(opened, hasLength(beforeBestie + 1));
          expect(opened.last, bestie);
        case HomeScreenShortcut.newDm:
          expect(dmStates.length, greaterThan(beforeDm));
          expect(nativeSession, isNotNull);
          await tester.binding.defaultBinaryMessenger.handlePlatformMessage(
            dmBridge.name,
            const StandardMethodCodec().encodeMethodCall(
              MethodCall('closed', {'session': nativeSession}),
            ),
            (_) {},
          );
          await tester.pumpAndSettle();
      }
    }

    // A real page veto, including one under another nested page, must not be
    // mistaken for the pane's navigation-only Back interception.
    await openSearchResult();
    final nested = Navigator.of(tester.element(find.byType(ChannelDetailPage)));
    nested.push(
      MaterialPageRoute<void>(
        builder: (_) => const ProfileEditPage(startInPhotoEditor: true),
      ),
    );
    await tester.pumpAndSettle();
    nested.push(
      MaterialPageRoute<void>(
        builder: (_) => const Scaffold(body: Text('Above editor')),
      ),
    );
    await tester.pumpAndSettle();
    final beforeVeto = opened.length;
    await tap('message-bestie');
    expect(find.text('Above editor'), findsNothing);
    expect(
      find.byKey(const ValueKey('profile-avatar-editor-page')),
      findsOneWidget,
    );
    expect(opened, hasLength(beforeVeto));
    expect(pendingHomeScreenShortcut.value, isNull);
    await nested.maybePop();
    await tester.pumpAndSettle();
    expect(find.byType(ProfileEditPage), findsNothing);
    await tap('message-bestie');
    expect(find.byType(ChannelDetailPage), findsNothing);
    expect(find.byType(SearchPage), findsNothing);
    expect(opened, hasLength(beforeVeto + 1));
    final completedOpens = List<String>.of(opened);

    // Shortcut state rebuilds only its own leaf, never Home.
    final builds = homeBuilds;
    tester.binding.handleAppLifecycleStateChanged(AppLifecycleState.inactive);
    await tester.pumpAndSettle();
    tester.binding.handleAppLifecycleStateChanged(AppLifecycleState.resumed);
    await tester.pumpAndSettle();
    expect(homeBuilds, builds);

    // A tap waiting on an unavailable Bestie returns to Home once, never
    // closing screens opened while it waits.
    bestieUpdates.addError(StateError('Bestie sync stopped'));
    await tester.pumpAndSettle();
    await tap('message-bestie');
    Navigator.of(
      tester.element(find.byType(HomePage)),
    ).push(MaterialPageRoute<void>(builder: (_) => const Text('Opened later')));
    await tester.pumpAndSettle();
    tester.binding.handleAppLifecycleStateChanged(AppLifecycleState.inactive);
    await tester.pumpAndSettle();
    tester.binding.handleAppLifecycleStateChanged(AppLifecycleState.resumed);
    await tester.pumpAndSettle();
    expect(find.text('Opened later'), findsOneWidget);
    // Once the Bestie is back, the stale tap opens nothing, even after the
    // newer screen closes.
    bestieUpdates.add(bestie);
    await tester.pumpAndSettle();
    expect(find.text('Opened later'), findsOneWidget);
    Navigator.of(
      tester.element(find.byType(HomePage, skipOffstage: false)),
    ).pop();
    await tester.pumpAndSettle();
    expect(opened, completedOpens);
    expect(pendingHomeScreenShortcut.value, isNull);

    // Signing out drops a tap that is still waiting, so it can't run in the
    // next account's Home.
    tester.binding.handleAppLifecycleStateChanged(AppLifecycleState.inactive);
    await tap('new-chat');
    await tester.pumpWidget(const SizedBox());
    expect(pendingHomeScreenShortcut.value, isNull);
    expect(published.last, isEmpty);
    pendingHomeScreenShortcut.value = null;
  }, variant: TargetPlatformVariant.only(TargetPlatform.iOS));

  for (final platform in [TargetPlatform.android, TargetPlatform.iOS]) {
    testWidgets('$platform scrolled bell opens Activity', (tester) async {
      debugDefaultTargetPlatformOverride = platform;
      addTearDown(() => debugDefaultTargetPlatformOverride = null);
      tester.view.physicalSize = const Size(390, 844);
      tester.view.devicePixelRatio = 1;
      addTearDown(tester.view.reset);
      await tester.pumpWidget(await buildHome(longSolo: true));
      await tester.pumpAndSettle();
      final scroll = tester
          .widget<CustomScrollView>(find.byType(CustomScrollView).first)
          .controller!;
      final collapseHeight = tester
          .widget<FrostedAppBar>(find.byType(FrostedAppBar))
          .bottomHeight;
      final title = find.text('Messages');
      final expandedCenter = platform == TargetPlatform.android
          ? tester.getCenter(title)
          : Offset.zero;
      for (final fraction in [0.01, 0.25, 0.5, 0.75, 1.0]) {
        scroll.jumpTo(collapseHeight * fraction);
        await tester.pumpAndSettle();
        expect(
          find.byTooltip('Activity').hitTestable(),
          findsOneWidget,
          reason: 'Bell must remain tappable at collapse fraction $fraction',
        );
      }
      if (platform == TargetPlatform.android) {
        final compactCenter = tester.getCenter(title);
        expect(compactCenter.dx, closeTo(195, 1));
        expect(compactCenter.dy, lessThan(expandedCenter.dy));
        scroll.jumpTo(0);
        await tester.pumpAndSettle();
        expect(tester.getCenter(title), expandedCenter);
        scroll.jumpTo(collapseHeight);
        await tester.pumpAndSettle();
      }
      if (platform == TargetPlatform.iOS) {
        // Widget tests hit-test the platform view but cannot run UIKit's
        // button action. Exercise the callback that native taps dispatch.
        tester
            .widget<IosGlassNavigationButton>(
              find.byWidgetPredicate(
                (widget) =>
                    widget is IosGlassNavigationButton &&
                    widget.icon == IosGlassNavigationIcon.bell,
              ),
            )
            .onPressed!();
      } else {
        await tester.tap(find.byTooltip('Activity'));
      }
      await tester.pumpAndSettle();
      expect(find.byType(ActivityPage), findsOneWidget);
      debugDefaultTargetPlatformOverride = null;
    });

    testWidgets('$platform bell follows unread inbox state', (tester) async {
      debugDefaultTargetPlatformOverride = platform;
      addTearDown(() => debugDefaultTargetPlatformOverride = null);
      for (final unread in [false, true, false]) {
        await tester.pumpWidget(await buildHome(inboxUnread: unread));
        await tester.pumpAndSettle();
        expect(
          tester.getCenter(find.byKey(const Key('home-inbox-dot'))) -
              tester.getTopLeft(find.byKey(const Key('home-inbox-badge'))),
          const Offset(17, 5),
        );
        final opacity = tester.widget<AnimatedOpacity>(
          find.ancestor(
            of: find.byKey(const Key('home-inbox-badge')),
            matching: find.byType(AnimatedOpacity),
          ),
        );
        expect(opacity.opacity, unread ? 1 : 0);
        expect(opacity.duration, const Duration(milliseconds: 150));
        expect(
          tester.getSize(find.byKey(const Key('home-inbox-badge'))),
          const Size.square(24),
        );
        expect(
          find.byTooltip(unread ? 'Activity, unread items' : 'Activity'),
          findsOneWidget,
        );
      }
      await tester.pumpWidget(
        await buildHome(inboxUnread: true, disableAnimations: true),
      );
      await tester.pump();
      expect(
        tester
            .widget<AnimatedOpacity>(
              find.ancestor(
                of: find.byKey(const Key('home-inbox-badge')),
                matching: find.byType(AnimatedOpacity),
              ),
            )
            .duration,
        Duration.zero,
      );
      debugDefaultTargetPlatformOverride = null;
    });
  }

  testWidgets(
    'iOS shows Messages above filters with the native creation menu',
    (tester) async {
      debugDefaultTargetPlatformOverride = TargetPlatform.iOS;
      addTearDown(() => debugDefaultTargetPlatformOverride = null);
      await tester.pumpWidget(await buildHome());
      await tester.pumpAndSettle();
      final bar = tester.widget<FrostedAppBar>(find.byType(FrostedAppBar));
      expect(bar.nativeTitle, 'Messages');
      expect(
        tester
            .widget<IosNavigationBar>(
              find.byWidgetPredicate(
                (w) => w is IosNavigationBar && w.title == 'Messages',
              ),
            )
            .externalExpandedTitle,
        isFalse,
      );
      expect(bar.nativeLargeTitle, isTrue);
      expect(
        find.byWidgetPredicate(
          (w) => w is IosNavigationBar && w.title == 'Messages',
        ),
        findsOneWidget,
      );
      expect(bar.nativeActions, isEmpty);
      expect(find.byTooltip('Messages options'), findsOneWidget);
      expect(
        tester
            .widget<IosMessageCreateButton>(
              find.byWidgetPredicate(
                (w) => w is IosMessageCreateButton && w.label == 'New',
              ),
            )
            .actions
            .map((a) => a.label),
        ['New chat', 'New channel', 'New DM', 'New workspace'],
      );
      final creationMenu = tester.widget<IosMessageCreateButton>(
        find.byWidgetPredicate(
          (w) => w is IosMessageCreateButton && w.label == 'New',
        ),
      );
      final search = find.byTooltip('Search');
      final shortcuts = [search, find.byType(DefaultAgentLauncher)];
      Iterable<double> shortcutOpacities() => shortcuts.map((shortcut) {
        final fade = find
            .ancestor(of: shortcut, matching: find.byType(FadeTransition))
            .first;
        return tester.widget<FadeTransition>(fade).opacity.value;
      });
      final initialX = tester.getCenter(search).dx;
      final initialWidth =
          (tester.getTopRight(search).dx - tester.getTopLeft(search).dx);
      creationMenu.onMenuVisibilityChanged!(true);
      await tester.pump();
      await tester.pump(const Duration(milliseconds: 90));
      expect(shortcutOpacities(), everyElement(inExclusiveRange(0, 1)));
      final midwayX = tester.getCenter(search).dx;
      expect(midwayX, greaterThan(initialX + 50));
      expect(midwayX, lessThan(initialX + 100));
      await tester.pumpAndSettle();
      expect(tester.getCenter(search).dx, closeTo(initialX + 100, .01));
      expect(
        (tester.getTopRight(search).dx - tester.getTopLeft(search).dx),
        closeTo(initialWidth * .9, .01),
      );
      expect(shortcutOpacities(), [0.0, 0.0]);
      expect(search.hitTestable(), findsNothing);
      creationMenu.onMenuVisibilityChanged!(false);
      await tester.pumpAndSettle();
      expect(tester.getCenter(search).dx, closeTo(initialX, .01));
      expect(
        (tester.getTopRight(search).dx - tester.getTopLeft(search).dx),
        closeTo(initialWidth, .01),
      );
      expect(shortcutOpacities(), [1.0, 1.0]);
      expect(search.hitTestable(), findsOneWidget);
      final dispatcher = tester.platformDispatcher;
      dispatcher.accessibilityFeaturesTestValue =
          const FakeAccessibilityFeatures(reduceMotion: true);
      addTearDown(dispatcher.clearAccessibilityFeaturesTestValue);
      expect(MediaQuery.disableAnimationsOf(tester.element(search)), isFalse);
      for (final open in [true, false]) {
        creationMenu.onMenuVisibilityChanged!(open);
        await tester.pump();
        expect(shortcutOpacities(), everyElement(open ? 0.0 : 1.0));
        expect(
          tester.getCenter(search).dx,
          closeTo(initialX + (open ? 100 : 0), .01),
        );
      }
      final inbox = find.byWidgetPredicate(
        (w) => w is IosGlassNavigationButton && w.semanticLabel == 'Activity',
      );
      expect(inbox, findsOneWidget);
      expect(find.text('Activity'), findsNothing);
      final inboxButton = tester.widget<IosGlassNavigationButton>(inbox);
      expect(inboxButton.icon, IosGlassNavigationIcon.bell);

      expect(tester.getSize(inbox), const Size(44, 44));
      final communityTrigger = find.byWidgetPredicate(
        (w) =>
            w is IosMessageCreateButton &&
            w.accessibilityLabel == 'Community menu',
      );
      expect(
        tester.getRect(inbox).right,
        lessThan(tester.getRect(communityTrigger).left),
      );
      inboxButton.onPressed!();
      await tester.pumpAndSettle();
      expect(find.byType(ActivityPage), findsOneWidget);
      expect(
        find.byWidgetPredicate(
          (w) => w is IosGlassNavigationButton && w.semanticLabel == 'Activity',
          skipOffstage: false,
        ),
        findsOneWidget,
      );
      Navigator.of(tester.element(find.byType(ActivityPage))).pop();
      await tester.pumpAndSettle();
      final communityMenu = tester.widget<IosMessageCreateButton>(
        find.byWidgetPredicate(
          (w) =>
              w is IosMessageCreateButton &&
              w.accessibilityLabel == 'Community menu',
        ),
      );
      communityMenu.actions.last.onPressed!();
      await tester.pump();
      expect(
        find.descendant(
          of: find.byType(IosMessageCreateButton, skipOffstage: false),
          matching: find.byType(UiKitView, skipOffstage: false),
        ),
        findsNothing,
      );
      await tester.pumpAndSettle();
      Navigator.of(tester.element(find.text('Your profile destination'))).pop();
      await tester.pump();
      expect(
        find.descendant(
          of: find.byType(IosMessageCreateButton, skipOffstage: false),
          matching: find.byType(UiKitView, skipOffstage: false),
        ),
        findsNothing,
      );
      await tester.pumpAndSettle();
      expect(
        find.descendant(
          of: find.byType(IosMessageCreateButton),
          matching: find.byType(UiKitView),
        ),
        findsNWidgets(2),
      );
      communityMenu.actions.first.onPressed!();
      await tester.pump();
      // Finish the route entrance while the community inventory is loading.
      await tester.pump(const Duration(milliseconds: 150));
      expect(find.text('Switch Community'), findsOneWidget);
      final homeOpacity = find.byKey(
        const Key('home-community-surface-visibility'),
      );
      expect(tester.widget<Opacity>(homeOpacity).opacity, 0);
      Navigator.of(tester.element(find.text('Switch Community'))).pop();
      await tester.pumpAndSettle();
      expect(tester.widget<Opacity>(homeOpacity).opacity, 1);
      expect(
        find.byWidgetPredicate(
          (w) => w is IosNavigationBar && w.title == 'Messages',
        ),
        findsOneWidget,
      );
      // A sheet's dim barrier can't composite over the native header edge.
      final headerNativeView = find.descendant(
        of: find.byType(FrostedAppBar),
        matching: find.byType(IosNavigationBar),
      );
      expect(headerNativeView, findsOneWidget);
      const bridge = MethodChannel('buzz/channel_actions_sheet');
      final selection = Completer<String?>();
      final calls = <MethodCall>[];
      tester.binding.defaultBinaryMessenger.setMockMethodCallHandler(bridge, (
        call,
      ) async {
        calls.add(call);
        return call.method == 'present' ? selection.future : null;
      });
      addTearDown(
        () => tester.binding.defaultBinaryMessenger.setMockMethodCallHandler(
          bridge,
          null,
        ),
      );
      await tester.longPress(find.text('General'));
      await tester.pumpAndSettle();
      expect(find.text('Mark unread'), findsOneWidget);
      expect(find.byType(BottomSheet), findsNothing);
      await tester.tapAt(const Offset(20, 20));
      await tester.pumpAndSettle();
      expect(find.text('Mark unread'), findsNothing);
      expect(headerNativeView, findsOneWidget);
      await tester.pumpWidget(const SizedBox.shrink());
      debugDefaultTargetPlatformOverride = null;
    },
  );
}

Widget _buildSettingsPage(BuildContext context) =>
    const Scaffold(body: Text('Your profile destination'));

class _Channels extends ChannelsNotifier {
  @override
  Future<List<Channel>> build() async => [
    Channel(
      id: 'channel',
      name: 'General',
      description: '',
      createdBy: 'owner',
      createdAt: DateTime(2026),
      memberCount: 1,
      visibility: 'open',
      channelType: 'stream',
      isMember: true,
    ),
  ];
}

class _Lifecycle extends AppLifecycleNotifier {
  @override
  AppLifecycleState build() => AppLifecycleState.resumed;
}
