import 'dart:async';
import 'dart:ui' show Tristate;

import 'package:buzz/features/home/home_page.dart';
import 'package:buzz/features/activity/activity_provider.dart';
import 'package:buzz/features/activity/feed_item.dart';
import 'package:buzz/features/channels/channel.dart';
import 'package:buzz/features/channels/channels_page.dart';
import 'package:buzz/features/channels/channels_provider.dart';
import 'package:buzz/features/profile/profile_avatar.dart';
import 'package:buzz/shared/relay/user_status.dart';
import 'package:buzz/shared/relay/user_status_provider.dart';
import 'package:buzz/shared/community/community.dart';
import 'package:buzz/shared/community/community_provider.dart';
import 'package:buzz/shared/theme/theme.dart';
import 'package:buzz/shared/widgets/skeleton.dart';
import 'package:flutter/material.dart';
import 'package:flutter/services.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';
import 'package:lucide_icons_flutter/lucide_icons.dart';
import 'package:shared_preferences/shared_preferences.dart';

void main() {
  Future<Widget> buildHome({
    ThemeData? theme,
    UserStatus? status,
    CommunityListNotifier? communityListNotifier,
    Community? activeCommunity,
    List<Channel>? channels,
    ActivityNotifier Function()? activityNotifier,
    bool usesMutableActiveCommunity = false,
  }) async {
    SharedPreferences.setMockInitialValues({});
    final prefs = await SharedPreferences.getInstance();
    return ProviderScope(
      overrides: [
        savedPrefsProvider.overrideWithValue(prefs),
        activityProvider.overrideWith(
          activityNotifier ?? _FakeActivityNotifier.new,
        ),
        if (status != null)
          userStatusProvider.overrideWith(
            () => _FakeUserStatusNotifier(status),
          ),
        if (communityListNotifier != null)
          communityListProvider.overrideWith(() => communityListNotifier),
        if (usesMutableActiveCommunity)
          activeCommunityProvider.overrideWith(
            (ref) async => ref.watch(_mutableActiveCommunityProvider),
          )
        else if (activeCommunity != null)
          activeCommunityProvider.overrideWith((ref) async => activeCommunity),
        if (channels != null)
          channelsProvider.overrideWith(() => _FakeChannelsNotifier(channels)),
      ],
      child: MaterialApp(
        theme: theme ?? AppTheme.light(),
        home: const HomePage(settingsPageBuilder: _buildSettingsPage),
      ),
    );
  }

  testWidgets('shows icon-only navigation and an aligned quick action', (
    tester,
  ) async {
    await tester.pumpWidget(await buildHome());
    await tester.pump();

    expect(find.text('Home'), findsNothing);
    expect(find.text('Activity'), findsNothing);
    expect(find.text('Search'), findsNothing);
    expect(find.bySemanticsLabel('Home'), findsOneWidget);
    expect(find.bySemanticsLabel('Activity'), findsOneWidget);
    expect(find.bySemanticsLabel('Search'), findsOneWidget);

    final quickAction = find.byTooltip('Create or start conversation');
    expect(quickAction, findsOneWidget);
    final launcherSize = tester.getSize(
      find.byType(ChannelQuickActionsLauncher),
    );
    expect(launcherSize.width, 800);
    expect(launcherSize.height, greaterThan(0));
    final motionRect = tester.getRect(
      find.byKey(const Key('channel-quick-actions-motion')),
    );
    expect(motionRect.width, const Size.square(56).width);
    expect(motionRect.left, greaterThanOrEqualTo(0));
    expect(tester.getSize(quickAction), const Size.square(56));
    final quickActionRect = tester.getRect(quickAction);
    expect(quickActionRect.left, greaterThanOrEqualTo(0));
    expect(quickActionRect.top, greaterThanOrEqualTo(0));
    expect(quickActionRect.right, lessThanOrEqualTo(800));
    expect(quickActionRect.bottom, lessThanOrEqualTo(600));
    final homeDestinationRect = tester.getRect(find.bySemanticsLabel('Home'));
    expect(
      quickActionRect.center.dy,
      closeTo(homeDestinationRect.center.dy, 0.01),
    );
  });

  testWidgets('uses a persistent sidebar instead of tabs on wide displays', (
    tester,
  ) async {
    tester.view.physicalSize = const Size(1180, 820);
    tester.view.devicePixelRatio = 1;
    addTearDown(() {
      tester.view.resetPhysicalSize();
      tester.view.resetDevicePixelRatio();
    });

    await tester.pumpWidget(await buildHome());
    await tester.pump();

    final sidebar = find.byKey(const Key('wide-navigation-sidebar'));
    final contentSurface = find.byKey(
      const Key('wide-navigation-content-surface'),
    );
    final inbox = find.byKey(const Key('wide-navigation-inbox'));
    final search = find.byKey(const Key('wide-navigation-search'));
    final communitySwitcher = find.byKey(
      const Key('wide-navigation-community-switcher'),
    );
    final communityIcon = find.byKey(
      const Key('wide-navigation-community-icon'),
    );
    final profile = find.byKey(const Key('wide-navigation-profile'));
    final sidebarSkeleton = find.byKey(
      const Key('wide-navigation-community-switch-skeleton'),
    );

    expect(sidebar, findsOneWidget);
    expect(
      tester
          .widget<SkeletonShimmer>(
            find.ancestor(
              of: sidebarSkeleton,
              matching: find.byType(SkeletonShimmer),
            ),
          )
          .enabled,
      isFalse,
    );
    expect(tester.getSize(sidebar).width, 280);
    expect(tester.getRect(contentSurface).left, 280);
    expect(tester.getRect(contentSurface).top, Grid.half + Grid.quarter);
    expect(
      tester.getRect(contentSurface).right,
      1180 - Grid.half - Grid.quarter,
    );
    expect(
      tester.getRect(contentSurface).bottom,
      820 - Grid.half - Grid.quarter,
    );
    final contentDecoration =
        tester.widget<DecoratedBox>(contentSurface).decoration as BoxDecoration;
    expect(contentDecoration.color, AppTheme.light().colorScheme.surface);
    expect(contentDecoration.borderRadius, BorderRadius.circular(24));
    expect(contentDecoration.boxShadow, hasLength(2));
    expect(contentDecoration.boxShadow!.first.offset, const Offset(-1, -1));
    expect(contentDecoration.boxShadow!.first.blurRadius, 0);
    expect(contentDecoration.boxShadow!.last.blurRadius, 4);
    final sidebarScrollView = find.byKey(
      const Key('wide-navigation-channel-list'),
    );
    expect(
      tester.getRect(sidebarScrollView).bottom,
      tester.getRect(profile).top - Grid.xxs,
    );
    expect(
      tester.getRect(profile).bottom,
      tester.getRect(sidebar).bottom - Grid.xs,
    );
    expect(communitySwitcher, findsOneWidget);
    expect(communityIcon, findsOneWidget);
    expect(tester.getSize(communityIcon), const Size.square(32));
    expect(tester.getSize(communitySwitcher).height, 48);
    expect(tester.getRect(communitySwitcher).top, 0);
    expect(profile, findsOneWidget);
    expect(
      tester
          .widget<ProfileAvatar>(
            find.descendant(of: profile, matching: find.byType(ProfileAvatar)),
          )
          .size,
      32,
    );
    expect(
      tester
          .getRect(
            find.descendant(
              of: communitySwitcher,
              matching: find.text('Community'),
            ),
          )
          .left,
      closeTo(
        tester
            .getRect(find.descendant(of: inbox, matching: find.text('Inbox')))
            .left,
        0.01,
      ),
    );
    final communityLabel = tester.widget<Text>(
      find.descendant(of: communitySwitcher, matching: find.text('Community')),
    );
    final inboxLabel = tester.widget<Text>(
      find.descendant(of: inbox, matching: find.text('Inbox')),
    );
    expect(communityLabel.style?.fontSize, 18);
    expect(
      communityLabel.style!.fontSize!,
      greaterThan(inboxLabel.style!.fontSize!),
    );
    expect(tester.getSize(inbox).height, greaterThanOrEqualTo(48));
    expect(tester.getSize(search).height, greaterThanOrEqualTo(48));
    expect(tester.getRect(search).top, lessThan(tester.getRect(inbox).top));
    final searchRow = find.descendant(
      of: search,
      matching: find.byType(Material),
    );
    final inboxRow = find.descendant(
      of: inbox,
      matching: find.byType(Material),
    );
    expect(
      tester.getRect(inboxRow).top - tester.getRect(searchRow).bottom,
      Grid.quarter,
    );
    expect(
      tester
          .widget<Icon>(find.descendant(of: inbox, matching: find.byType(Icon)))
          .size,
      20,
    );
    expect(
      tester
          .widget<Icon>(find.descendant(of: inbox, matching: find.byType(Icon)))
          .icon,
      LucideIcons.inbox500,
    );
    expect(
      tester
          .widget<Icon>(
            find.descendant(of: search, matching: find.byType(Icon)),
          )
          .size,
      20,
    );
    expect(
      tester
          .widget<Icon>(
            find.descendant(of: search, matching: find.byType(Icon)),
          )
          .icon,
      LucideIcons.search500,
    );
    expect(find.byKey(const Key('floating-tab-bar')), findsNothing);
    expect(find.byKey(const Key('wide-navigation-home')), findsNothing);
    expect(search, findsOneWidget);
    expect(find.byTooltip('Create channel'), findsOneWidget);
    expect(find.byTooltip('New direct message'), findsOneWidget);
    expect(find.byTooltip('Create or start conversation'), findsNothing);
    expect(
      tester.getSemantics(inbox).flagsCollection.isSelected,
      Tristate.isTrue,
    );
    expect(
      tester
          .widget<IndexedStack>(
            find.descendant(
              of: contentSurface,
              matching: find.byType(IndexedStack),
            ),
          )
          .index,
      1,
    );

    await tester.tap(inbox);
    await tester.pump();

    expect(
      tester.getSemantics(inbox).flagsCollection.isSelected,
      Tristate.isTrue,
    );

    await tester.tap(search);
    await tester.pump();

    expect(find.byKey(const Key('search-field')), findsOneWidget);

    tester
        .widget<InkWell>(
          find.byKey(const Key('wide-navigation-profile-button')),
        )
        .onTap!();
    await tester.pump(const Duration(milliseconds: 300));

    expect(Navigator.of(tester.element(profile)).canPop(), isTrue);

    Navigator.of(tester.element(profile)).pop();
    await tester.pump();

    await tester.tap(find.byKey(const Key('wide-navigation-settings-button')));
    await tester.pump(const Duration(milliseconds: 300));

    expect(Navigator.of(tester.element(profile)).canPop(), isTrue);

    Navigator.of(tester.element(profile)).pop();
    await tester.pump();

    await tester.tap(communitySwitcher);
    await tester.pump();
    expect(find.byKey(const Key('community-switcher-sheet')), findsOneWidget);
  });

  testWidgets('clears a selected tablet channel when the community changes', (
    tester,
  ) async {
    tester.view.physicalSize = const Size(1180, 820);
    tester.view.devicePixelRatio = 1;
    addTearDown(() {
      tester.view.resetPhysicalSize();
      tester.view.resetDevicePixelRatio();
    });

    await tester.pumpWidget(
      await buildHome(
        usesMutableActiveCommunity: true,
        channels: [
          Channel(
            id: 'alpha-channel',
            name: 'alpha',
            channelType: 'stream',
            visibility: 'open',
            description: '',
            createdBy: 'me',
            createdAt: DateTime(2026),
            memberCount: 1,
            isMember: true,
          ),
        ],
      ),
    );
    final container = ProviderScope.containerOf(
      tester.element(find.byType(HomePage)),
    );
    container
        .read(_mutableActiveCommunityProvider.notifier)
        .select(
          Community(
            id: 'alpha',
            name: 'Alpha',
            relayUrl: 'wss://alpha.example.com',
            addedAt: DateTime(2026),
          ),
        );
    await tester.pumpAndSettle();
    await tester.tap(find.byKey(const ValueKey('wide-channel-alpha-channel')));
    await tester.pump();

    expect(
      tester
          .getSemantics(
            find.byKey(const ValueKey('wide-channel-alpha-channel')),
          )
          .flagsCollection
          .isSelected,
      Tristate.isTrue,
    );

    container
        .read(_mutableActiveCommunityProvider.notifier)
        .select(
          Community(
            id: 'bravo',
            name: 'Bravo',
            relayUrl: 'wss://bravo.example.com',
            addedAt: DateTime(2026),
          ),
        );
    await tester.pump();

    expect(
      tester
          .getSemantics(
            find.byKey(const ValueKey('wide-channel-alpha-channel')),
          )
          .flagsCollection
          .isSelected,
      Tristate.isFalse,
    );
    expect(find.byKey(const Key('wide-navigation-inbox')), findsOneWidget);
  });

  testWidgets('extends the Buzz gradient through the iPad sidebar', (
    tester,
  ) async {
    tester.view.physicalSize = const Size(1180, 820);
    tester.view.devicePixelRatio = 1;
    addTearDown(() {
      tester.view.resetPhysicalSize();
      tester.view.resetDevicePixelRatio();
    });

    final gradient = const LinearGradient(colors: [Colors.yellow, Colors.blue]);
    await tester.pumpWidget(
      await buildHome(theme: AppTheme.light(topSectionGradient: gradient)),
    );
    await tester.pump();

    final sidebar = tester.widget<DecoratedBox>(
      find.byKey(const Key('wide-navigation-sidebar-background')),
    );
    final decoration = sidebar.decoration as BoxDecoration;
    expect(decoration.gradient, same(gradient));
    expect(decoration.color, isNull);
    expect(decoration.border, isNull);

    final foreground = Theme.of(
      tester.element(find.byKey(const Key('wide-navigation-sidebar'))),
    ).colorScheme.onSurface;
    final inbox = find.byKey(const Key('wide-navigation-inbox'));
    final inboxLabel = tester.widget<Text>(
      find.descendant(of: inbox, matching: find.text('Inbox')),
    );
    final inboxMaterial = tester.widget<Material>(
      find.descendant(of: inbox, matching: find.byType(Material)),
    );
    final channelsLabel = tester.widget<Text>(find.text('Channels'));

    expect(inboxLabel.style?.color, foreground);
    expect(inboxMaterial.color, foreground.withValues(alpha: 0.07));
    expect(channelsLabel.style?.color, foreground.withValues(alpha: 0.4));
  });

  testWidgets('selecting Inbox or Search clears a selected tablet channel', (
    tester,
  ) async {
    tester.view.physicalSize = const Size(1180, 820);
    tester.view.devicePixelRatio = 1;
    addTearDown(() {
      tester.view.resetPhysicalSize();
      tester.view.resetDevicePixelRatio();
    });
    final channel = Channel(
      id: 'test-channel',
      name: 'general',
      channelType: 'stream',
      visibility: 'open',
      description: '',
      createdBy: 'alice',
      createdAt: DateTime(2025),
      memberCount: 1,
    );

    await tester.pumpWidget(await buildHome(channels: [channel]));
    await tester.pumpAndSettle();

    final channelDestination = find.byKey(
      const ValueKey('wide-channel-test-channel'),
    );
    final inbox = find.byKey(const Key('wide-navigation-inbox'));
    final search = find.byKey(const Key('wide-navigation-search'));

    await tester.tap(channelDestination);
    await tester.pump();
    expect(
      tester.getSemantics(channelDestination).flagsCollection.isSelected,
      Tristate.isTrue,
    );

    await tester.tap(inbox);
    await tester.pump();
    expect(
      tester.getSemantics(inbox).flagsCollection.isSelected,
      Tristate.isTrue,
    );
    expect(
      tester.getSemantics(channelDestination).flagsCollection.isSelected,
      Tristate.isFalse,
    );

    await tester.tap(channelDestination);
    await tester.pump();
    await tester.tap(search);
    await tester.pump();
    expect(
      tester.getSemantics(search).flagsCollection.isSelected,
      Tristate.isTrue,
    );
    expect(
      tester.getSemantics(channelDestination).flagsCollection.isSelected,
      Tristate.isFalse,
    );
  });

  testWidgets('shows the current status in the wide sidebar profile row', (
    tester,
  ) async {
    tester.view.physicalSize = const Size(1180, 820);
    tester.view.devicePixelRatio = 1;
    addTearDown(() {
      tester.view.resetPhysicalSize();
      tester.view.resetDevicePixelRatio();
    });

    await tester.pumpWidget(
      await buildHome(
        status: const UserStatus(text: 'Focusing', emoji: '🎯', updatedAt: 1),
      ),
    );
    await tester.pump();
    await tester.pump();

    expect(
      find.descendant(
        of: find.byKey(const Key('wide-navigation-profile')),
        matching: find.text('🎯 Focusing'),
      ),
      findsOneWidget,
    );
  });

  testWidgets('swiping the community icon selects the next community', (
    tester,
  ) async {
    tester.view.physicalSize = const Size(1180, 820);
    tester.view.devicePixelRatio = 1;
    addTearDown(() {
      tester.view.resetPhysicalSize();
      tester.view.resetDevicePixelRatio();
    });

    final communities = [
      Community(
        id: 'alpha',
        name: 'Alpha',
        relayUrl: 'wss://alpha.example.com',
        addedAt: DateTime(2026),
      ),
      Community(
        id: 'bravo',
        name: 'Bravo',
        relayUrl: 'wss://bravo.example.com',
        addedAt: DateTime(2026),
      ),
    ];
    final communityListNotifier = _FakeCommunityListNotifier(communities);

    await tester.pumpWidget(
      await buildHome(
        communityListNotifier: communityListNotifier,
        activeCommunity: communities.first,
      ),
    );
    await tester.pump();
    await tester.pump(const Duration(milliseconds: 1));

    expect(find.text('Alpha'), findsOneWidget);
    expect(communityListNotifier.state.value, communities);
    expect(
      find.byKey(const Key('wide-navigation-community-current-icon')),
      findsOneWidget,
    );
    expect(
      find.byKey(const Key('wide-navigation-community-incoming-icon')),
      findsOneWidget,
    );
    final iconGesture = tester.widget<GestureDetector>(
      find.descendant(
        of: find.byKey(const Key('wide-navigation-community-icon')),
        matching: find.byType(GestureDetector),
      ),
    );
    iconGesture.onVerticalDragStart!(
      DragStartDetails(globalPosition: Offset.zero),
    );
    iconGesture.onVerticalDragUpdate!(
      DragUpdateDetails(globalPosition: Offset(0, 40), delta: Offset(0, 40)),
    );
    await tester.pump();
    final outgoingContainer = tester.widget<Transform>(
      find.byKey(const Key('wide-navigation-community-current-container')),
    );
    expect(outgoingContainer.transform.getTranslation().y, closeTo(32, 0.01));
    expect(
      tester
          .widget<Opacity>(
            find.byKey(const Key('wide-navigation-community-current-opacity')),
          )
          .opacity,
      0,
    );
    expect(
      tester
          .widget<Opacity>(
            find.byKey(const Key('wide-navigation-community-incoming-opacity')),
          )
          .opacity,
      0,
    );
    expect(
      find.byKey(const Key('wide-navigation-community-incoming-container')),
      findsOneWidget,
    );
    iconGesture.onVerticalDragEnd!(DragEndDetails(velocity: Velocity.zero));
    await tester.pump();
    await tester.pump(const Duration(milliseconds: 120));
    await tester.pump();

    expect(
      tester
          .widget<Opacity>(
            find.byKey(const Key('wide-navigation-community-incoming-opacity')),
          )
          .opacity,
      1,
    );
    expect(find.text('Bravo'), findsOneWidget);
    expect(
      tester
          .widget<SkeletonShimmer>(
            find.ancestor(
              of: find.byKey(
                const Key('wide-navigation-community-switch-skeleton'),
              ),
              matching: find.byType(SkeletonShimmer),
            ),
          )
          .enabled,
      isFalse,
    );

    await tester.pump(const Duration(milliseconds: 120));
    await tester.pump();

    expect(communityListNotifier.switchedToId, 'bravo');
    expect(
      tester
          .widget<SkeletonShimmer>(
            find.ancestor(
              of: find.byKey(
                const Key('wide-navigation-community-switch-skeleton'),
              ),
              matching: find.byType(SkeletonShimmer),
            ),
          )
          .enabled,
      isTrue,
    );
    expect(
      tester
          .widget<SkeletonShimmer>(
            find.ancestor(
              of: find.byKey(
                const Key('wide-community-switch-content-skeleton'),
              ),
              matching: find.byType(SkeletonShimmer),
            ),
          )
          .enabled,
      isTrue,
    );
  });

  testWidgets('shows the workspace skeleton while sheet switching community', (
    tester,
  ) async {
    tester.view.physicalSize = const Size(1180, 820);
    tester.view.devicePixelRatio = 1;
    addTearDown(() {
      tester.view.resetPhysicalSize();
      tester.view.resetDevicePixelRatio();
    });
    final communities = [
      Community(
        id: 'alpha',
        name: 'Alpha',
        relayUrl: 'wss://alpha.example.com',
        addedAt: DateTime(2026),
      ),
      Community(
        id: 'bravo',
        name: 'Bravo',
        relayUrl: 'wss://bravo.example.com',
        addedAt: DateTime(2026),
      ),
    ];
    final communityListNotifier = _FakeCommunityListNotifier(communities);

    await tester.pumpWidget(
      await buildHome(
        communityListNotifier: communityListNotifier,
        activeCommunity: communities.first,
      ),
    );
    await tester.pump();

    await tester.tap(
      find.byKey(const Key('wide-navigation-community-switcher')),
    );
    await tester.pump(const Duration(milliseconds: 300));
    tester
        .widget<InkWell>(
          find.ancestor(of: find.text('Bravo'), matching: find.byType(InkWell)),
        )
        .onTap!();
    await tester.pump();

    expect(communityListNotifier.switchedToId, 'bravo');
    expect(
      tester
          .widget<SkeletonShimmer>(
            find.ancestor(
              of: find.byKey(
                const Key('wide-navigation-community-switch-skeleton'),
              ),
              matching: find.byType(SkeletonShimmer),
            ),
          )
          .enabled,
      isTrue,
    );
  });

  testWidgets('keeps the workspace skeleton until Activity is ready', (
    tester,
  ) async {
    tester.view.physicalSize = const Size(1180, 820);
    tester.view.devicePixelRatio = 1;
    addTearDown(() {
      tester.view.resetPhysicalSize();
      tester.view.resetDevicePixelRatio();
    });
    final communities = [
      Community(
        id: 'alpha',
        name: 'Alpha',
        relayUrl: 'wss://alpha.example.com',
        addedAt: DateTime(2026),
      ),
      Community(
        id: 'bravo',
        name: 'Bravo',
        relayUrl: 'wss://bravo.example.com',
        addedAt: DateTime(2026),
      ),
    ];
    final communityListNotifier = _FakeCommunityListNotifier(communities);

    await tester.pumpWidget(
      await buildHome(
        activityNotifier: _PendingActivityNotifier.new,
        communityListNotifier: communityListNotifier,
        activeCommunity: communities.first,
      ),
    );
    await tester.pump();
    await tester.tap(
      find.byKey(const Key('wide-navigation-community-switcher')),
    );
    await tester.pump(const Duration(milliseconds: 300));
    tester
        .widget<InkWell>(
          find.ancestor(of: find.text('Bravo'), matching: find.byType(InkWell)),
        )
        .onTap!();
    await tester.pump(const Duration(milliseconds: 400));

    expect(
      tester
          .widget<SkeletonShimmer>(
            find.ancestor(
              of: find.byKey(
                const Key('wide-navigation-community-switch-skeleton'),
              ),
              matching: find.byType(SkeletonShimmer),
            ),
          )
          .enabled,
      isTrue,
    );
  });

  testWidgets('a failed community swipe restores the workspace', (
    tester,
  ) async {
    tester.view.physicalSize = const Size(1180, 820);
    tester.view.devicePixelRatio = 1;
    addTearDown(() {
      tester.view.resetPhysicalSize();
      tester.view.resetDevicePixelRatio();
    });
    final communityListNotifier = _FailingCommunityListNotifier([
      Community(
        id: 'alpha',
        name: 'Alpha',
        relayUrl: 'wss://alpha.example.com',
        addedAt: DateTime(2026),
      ),
      Community(
        id: 'bravo',
        name: 'Bravo',
        relayUrl: 'wss://bravo.example.com',
        addedAt: DateTime(2026),
      ),
    ]);

    await tester.pumpWidget(
      await buildHome(
        communityListNotifier: communityListNotifier,
        activeCommunity: communityListNotifier.communities.first,
      ),
    );
    await tester.pump();
    await tester.pump(const Duration(milliseconds: 1));

    Future<void> swipe() async {
      final gesture = tester.widget<GestureDetector>(
        find.descendant(
          of: find.byKey(const Key('wide-navigation-community-icon')),
          matching: find.byType(GestureDetector),
        ),
      );
      gesture.onVerticalDragStart!(
        DragStartDetails(globalPosition: Offset.zero),
      );
      gesture.onVerticalDragUpdate!(
        DragUpdateDetails(
          globalPosition: const Offset(0, 40),
          delta: const Offset(0, 40),
        ),
      );
      await tester.pump();
      gesture.onVerticalDragEnd!(DragEndDetails(velocity: Velocity.zero));
      await tester.pump();
      await tester.pump(const Duration(milliseconds: 120));
      await tester.pump();
      await tester.pump(const Duration(milliseconds: 120));
      await tester.pump();
    }

    await swipe();
    expect(communityListNotifier.switchAttempts, 1);
    expect(
      tester
          .widget<SkeletonShimmer>(
            find.ancestor(
              of: find.byKey(
                const Key('wide-navigation-community-switch-skeleton'),
              ),
              matching: find.byType(SkeletonShimmer),
            ),
          )
          .enabled,
      isFalse,
    );

    await swipe();
    expect(communityListNotifier.switchAttempts, 2);
  });

  testWidgets('gives selection haptics only when the tab changes', (
    tester,
  ) async {
    final hapticCalls = <MethodCall>[];
    TestDefaultBinaryMessengerBinding.instance.defaultBinaryMessenger
        .setMockMethodCallHandler(SystemChannels.platform, (call) async {
          if (call.method == 'HapticFeedback.vibrate') {
            hapticCalls.add(call);
          }
          return null;
        });
    addTearDown(
      () => TestDefaultBinaryMessengerBinding.instance.defaultBinaryMessenger
          .setMockMethodCallHandler(SystemChannels.platform, null),
    );

    await tester.pumpWidget(await buildHome());
    await tester.pump();

    await tester.tap(find.byTooltip('Home'));
    await tester.pump();
    expect(hapticCalls, isEmpty);

    await tester.tap(find.byTooltip('Activity'));
    await tester.pump();
    expect(hapticCalls, hasLength(1));
    expect(hapticCalls.single.arguments, 'HapticFeedbackType.selectionClick');

    await tester.tap(find.byTooltip('Activity'));
    await tester.pump();
    expect(hapticCalls, hasLength(1));

    await tester.tap(find.byTooltip('Search'));
    await tester.pump();
    expect(hapticCalls, hasLength(2));
  });

  testWidgets('scales and fades the quick action as tabs change', (
    tester,
  ) async {
    await tester.pumpWidget(await buildHome());
    await tester.pump();

    double scale() => tester
        .widget<Transform>(find.byKey(const Key('channel-quick-actions-scale')))
        .transform
        .storage
        .first;
    double opacity() => tester
        .widget<Opacity>(find.byKey(const Key('channel-quick-actions-opacity')))
        .opacity;

    expect(scale(), closeTo(1, 0.001));
    expect(opacity(), closeTo(1, 0.001));

    await tester.tap(find.byTooltip('Activity'));
    await tester.pump();
    await tester.pump(const Duration(milliseconds: 110));

    expect(scale(), inExclusiveRange(0.8, 1));
    expect(opacity(), inExclusiveRange(0, 1));

    await tester.pumpAndSettle();
    expect(scale(), closeTo(0.8, 0.001));
    expect(opacity(), closeTo(0, 0.001));

    await tester.tap(find.byTooltip('Home'));
    await tester.pump();
    await tester.pump(const Duration(milliseconds: 110));

    expect(scale(), inExclusiveRange(0.8, 1));
    expect(opacity(), inExclusiveRange(0, 1));

    await tester.pumpAndSettle();
    expect(scale(), closeTo(1, 0.001));
    expect(opacity(), closeTo(1, 0.001));
  });
}

Widget _buildSettingsPage(BuildContext context) => const SizedBox.shrink();

final _mutableActiveCommunityProvider =
    NotifierProvider<_MutableActiveCommunityNotifier, Community?>(
      _MutableActiveCommunityNotifier.new,
    );

class _MutableActiveCommunityNotifier extends Notifier<Community?> {
  @override
  Community? build() => null;

  void select(Community? community) => state = community;
}

class _FakeUserStatusNotifier extends UserStatusNotifier {
  _FakeUserStatusNotifier(this._status);

  final UserStatus _status;

  @override
  Future<UserStatus?> build() async => _status;
}

class _FakeChannelsNotifier extends ChannelsNotifier {
  final List<Channel> _channels;

  _FakeChannelsNotifier(this._channels);

  @override
  Future<List<Channel>> build() async => _channels;
}

class _FakeActivityNotifier extends ActivityNotifier {
  @override
  Future<HomeFeedResponse> build() async => HomeFeedResponse(
    mentions: const [],
    needsAction: const [],
    activity: const [],
    agentActivity: const [],
  );
}

class _PendingActivityNotifier extends ActivityNotifier {
  @override
  Future<HomeFeedResponse> build() => Completer<HomeFeedResponse>().future;
}

class _FakeCommunityListNotifier extends CommunityListNotifier {
  _FakeCommunityListNotifier(this._communities);

  final List<Community> _communities;
  String? switchedToId;

  @override
  Future<List<Community>> build() async => _communities;

  @override
  Future<void> switchCommunity(String id) async {
    switchedToId = id;
  }
}

class _FailingCommunityListNotifier extends CommunityListNotifier {
  _FailingCommunityListNotifier(this.communities);

  final List<Community> communities;
  int switchAttempts = 0;

  @override
  Future<List<Community>> build() async => communities;

  @override
  Future<void> switchCommunity(String id) async {
    switchAttempts++;
    throw Exception('Unable to save active community');
  }
}
