import 'dart:async';

import 'package:flutter/foundation.dart';
import 'package:buzz/features/activity/activity_page.dart';
import 'package:buzz/features/activity/activity_provider.dart';
import 'package:buzz/features/activity/feed_item.dart';
import 'package:buzz/features/activity/inbox_item.dart';
import 'package:buzz/features/activity/reminders_provider.dart';
import 'package:buzz/features/channels/channel.dart';
import 'package:buzz/features/channels/channel_detail_page.dart';
import 'package:buzz/features/channels/message_content.dart';
import 'package:buzz/features/channels/channels_provider.dart';
import 'package:buzz/shared/read_state/read_state_provider.dart';
import 'package:buzz/features/profile/user_cache_provider.dart';
import 'package:buzz/features/profile/user_profile.dart';
import 'package:buzz/shared/theme/theme.dart';
import 'package:buzz/shared/widgets/anchored_popover_menu.dart';
import 'package:buzz/shared/widgets/frosted_app_bar.dart';
import 'package:buzz/shared/widgets/avatar_image.dart';
import 'package:flutter/material.dart';
import 'package:flutter/services.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';
import 'package:lucide_icons_flutter/lucide_icons.dart';
import 'package:shared_preferences/shared_preferences.dart';

void main() {
  final now = DateTime.now().millisecondsSinceEpoch ~/ 1000;

  final testMention = FeedItem(
    id: 'm1',
    kind: 9,
    pubkey: 'alice_pk',
    content: 'Hey check this out',
    createdAt: now - 120,
    channelId: 'ch1',
    channelName: 'general',
    tags: const [],
    category: 'mention',
  );

  final testThreadReply = FeedItem(
    id: 'a1',
    kind: 9,
    pubkey: 'bob_pk',
    content: 'Deployed the fix',
    createdAt: now - 3600,
    channelId: 'ch2',
    channelName: 'engineering',
    tags: const [
      ['e', 'root1', '', 'root'],
      ['e', 'root1', '', 'reply'],
    ],
    category: 'activity',
  );

  final testAgent = FeedItem(
    id: 'ag1',
    kind: 43004,
    pubkey: 'agent_pk',
    content: 'Job completed successfully',
    createdAt: now - 60,
    channelId: 'ch1',
    channelName: 'general',
    tags: const [],
    category: 'agent_activity',
  );

  final testFeed = HomeFeedResponse(
    mentions: [testMention],
    needsAction: const [],
    activity: [testThreadReply],
    agentActivity: [testAgent],
  );

  final testChannels = [
    Channel(
      id: 'ch1',
      name: 'general',
      channelType: 'stream',
      visibility: 'open',
      description: '',
      createdBy: 'x',
      createdAt: DateTime(2025),
      memberCount: 5,
      isMember: true,
    ),
    Channel(
      id: 'ch2',
      name: 'engineering',
      channelType: 'stream',
      visibility: 'open',
      description: '',
      createdBy: 'x',
      createdAt: DateTime(2025),
      memberCount: 3,
      isMember: true,
    ),
  ];

  final testUsers = <String, UserProfile>{
    'alice_pk': const UserProfile(
      pubkey: 'alice_pk',
      displayName: 'Alice',
      nip05Handle: 'alice@example.com',
    ),
    'bob_pk': const UserProfile(pubkey: 'bob_pk', displayName: 'Bob'),
    'agent_pk': const UserProfile(pubkey: 'agent_pk', displayName: 'Scout'),
  };

  Future<Widget> buildTestable({
    HomeFeedResponse? feed,
    ActivityNotifier Function()? activityNotifier,
    Map<String, UserProfile>? users,
    Map<String, int> readContexts = const {},
    List<Channel>? channels,
    TextScaler? textScaler,
    EdgeInsets mediaPadding = EdgeInsets.zero,
    ValueListenable<int>? tabReselection,
  }) async {
    SharedPreferences.setMockInitialValues({});
    final prefs = await SharedPreferences.getInstance();
    return ProviderScope(
      overrides: [
        savedPrefsProvider.overrideWithValue(prefs),
        activityProvider.overrideWith(
          activityNotifier ?? () => _FakeActivityNotifier(feed ?? testFeed),
        ),
        channelsProvider.overrideWith(
          () => _FakeChannelsNotifier(channels ?? testChannels),
        ),
        userCacheProvider.overrideWith(
          () => _FakeUserCacheNotifier(users ?? testUsers),
        ),
        readStateProvider.overrideWith(
          () => _FakeReadStateNotifier(readContexts),
        ),
        remindersProvider.overrideWith(() => _FakeRemindersNotifier(const [])),
      ],
      child: MaterialApp(
        theme: AppTheme.light(),
        builder: (context, child) => MediaQuery(
          data: MediaQuery.of(
            context,
          ).copyWith(textScaler: textScaler, padding: mediaPadding),
          child: child!,
        ),
        home: ActivityPage(tabReselection: tabReselection),
      ),
    );
  }

  testWidgets('shows loading skeleton while feed loads', (tester) async {
    await tester.pumpWidget(
      await buildTestable(activityNotifier: _PendingActivityNotifier.new),
    );
    // Single pump - don't settle, the future never completes.
    await tester.pump();

    expect(find.byType(Container), findsWidgets);
    expect(find.text('Hey check this out'), findsNothing);
  });

  testWidgets('shows empty state when feed is empty', (tester) async {
    await tester.pumpWidget(
      await buildTestable(
        feed: HomeFeedResponse(
          mentions: const [],
          needsAction: const [],
          activity: const [],
          agentActivity: const [],
        ),
      ),
    );
    await tester.pumpAndSettle();

    expect(find.text('No activity yet'), findsOneWidget);
  });

  testWidgets('does not imply a back button for the top-level Activity tab', (
    tester,
  ) async {
    await tester.pumpWidget(await buildTestable());
    await tester.pumpAndSettle();

    final appBar = tester.widget<FrostedAppBar>(
      find.byType(FrostedAppBar).last,
    );
    expect(appBar.automaticallyImplyLeading, isFalse);
    expect(appBar.gradient, isNull);
    expect(appBar.frosted, isTrue);
    expect(appBar.showBottomDivider, isTrue);
    expect(appBar.bottomHeight, Grid.xxs);
    expect(find.byTooltip('Back'), findsNothing);
  });

  testWidgets('sizes the Activity app bar for its custom title style', (
    tester,
  ) async {
    await tester.pumpWidget(
      await buildTestable(textScaler: const TextScaler.linear(2)),
    );
    await tester.pumpAndSettle();

    final appBar = tester.widget<FrostedAppBar>(
      find.byType(FrostedAppBar).last,
    );
    final titleStyle = appBar.titleStyle!;
    expect(titleStyle.fontSize, 22);
    expect(
      tester.getSize(find.byType(ClipRect).last).height,
      closeTo(
        frostedAppBarHeight(
          tester.element(find.byType(FrostedAppBar).last),
          titleStyle: titleStyle,
          bottomHeight: Grid.xxs,
        ),
        0.01,
      ),
    );
  });

  testWidgets('keeps footer clearance inside the scrollable content', (
    tester,
  ) async {
    await tester.pumpWidget(
      await buildTestable(mediaPadding: const EdgeInsets.only(bottom: 88)),
    );
    await tester.pumpAndSettle();

    final safeArea = tester.widget<SafeArea>(
      find.byKey(const ValueKey('activity-content-safe-area')),
    );
    expect(safeArea.top, isFalse);
    expect(safeArea.bottom, isFalse);

    final padding = tester.widget<SliverPadding>(
      find.descendant(
        of: find.byType(CustomScrollView),
        matching: find.byType(SliverPadding),
      ),
    );
    expect(padding.padding, const EdgeInsets.fromLTRB(0, Grid.xxs, 0, 96));
  });

  testWidgets('scrolls Activity to the top when its tab is selected again', (
    tester,
  ) async {
    tester.view.physicalSize = const Size(320, 180);
    tester.view.devicePixelRatio = 1;
    addTearDown(tester.view.reset);
    final tabReselection = ValueNotifier(0);
    addTearDown(tabReselection.dispose);
    await tester.pumpWidget(
      await buildTestable(tabReselection: tabReselection),
    );
    await tester.pumpAndSettle();

    final scrollable = tester.state<ScrollableState>(
      find
          .descendant(
            of: find.byType(CustomScrollView),
            matching: find.byType(Scrollable),
          )
          .first,
    );
    expect(scrollable.position.maxScrollExtent, greaterThan(0));
    scrollable.position.jumpTo(scrollable.position.maxScrollExtent);
    tabReselection.value++;
    await tester.pump();
    await tester.pump(const Duration(milliseconds: 130));

    expect(
      scrollable.position.pixels,
      lessThan(scrollable.position.maxScrollExtent),
    );
    await tester.pumpAndSettle();
    expect(scrollable.position.pixels, scrollable.position.minScrollExtent);
  });

  testWidgets('shows error view with retry button', (tester) async {
    await tester.pumpWidget(
      await buildTestable(activityNotifier: _ErrorActivityNotifier.new),
    );
    await tester.pumpAndSettle();

    expect(find.text('Failed to load activity'), findsOneWidget);
    expect(find.text('Retry'), findsOneWidget);
  });

  testWidgets('activity popovers use fixed-layout scale and opacity motion', (
    tester,
  ) async {
    await tester.pumpWidget(await buildTestable());
    await tester.pumpAndSettle();

    final filterTrigger = find.byKey(const ValueKey('activity-filter-menu'));
    expect(
      tester.getSize(filterTrigger).height,
      greaterThanOrEqualTo(Grid.xl),
      reason: 'The Activity filter trigger must keep a 48dp touch target.',
    );

    await tester.tap(filterTrigger);
    await tester.pump();

    final surface = find.byKey(const ValueKey('activity-filter-popover'));
    final fade = find.byKey(const ValueKey('activity-popover-fade'));
    final scale = find.byKey(const ValueKey('activity-popover-scale'));
    expect(surface, findsOneWidget);
    expect(fade, findsOneWidget);
    expect(scale, findsOneWidget);

    final initialSize = tester.getSize(surface);
    final initialFade = tester.widget<FadeTransition>(fade);
    final initialScale = tester.widget<ScaleTransition>(scale);
    expect(initialSize.width, 240);
    expect(initialFade.opacity.value, lessThan(1));
    expect(initialScale.scale.value, greaterThanOrEqualTo(0.96));
    expect(initialScale.scale.value, lessThan(1));
    expect(initialScale.alignment, Alignment.topLeft);

    await tester.pump(const Duration(milliseconds: 75));

    final movingFade = tester.widget<FadeTransition>(fade);
    final movingScale = tester.widget<ScaleTransition>(scale);
    expect(movingFade.opacity.value, greaterThan(0));
    expect(movingFade.opacity.value, lessThan(1));
    expect(movingScale.scale.value, greaterThan(0.96));
    expect(movingScale.scale.value, lessThan(1));
    expect(tester.getSize(surface), initialSize);

    await tester.pump(const Duration(milliseconds: 75));
    expect(tester.widget<FadeTransition>(fade).opacity.value, 1);
    expect(tester.widget<ScaleTransition>(scale).scale.value, 1);
    expect(tester.getSize(surface), initialSize);

    final material = tester.widget<Material>(surface);
    final shape = material.shape! as RoundedRectangleBorder;
    expect(shape.borderRadius, BorderRadius.circular(Radii.popover));
    expect(shape.side.color, Colors.black.withValues(alpha: 0.04));
    expect(material.elevation, appPopoverElevation);
    expect(
      material.shadowColor,
      appPopoverShadowColor(tester.element(surface)),
    );
    expect(material.surfaceTintColor, Colors.transparent);
    expect(material.clipBehavior, Clip.antiAlias);

    final items = tester.widgetList<PopupMenuItem<InboxFilter>>(
      find.byType(PopupMenuItem<InboxFilter>),
    );
    expect(items, hasLength(InboxFilter.values.length));
    expect(
      items.every((item) => item.height >= Grid.xl),
      isTrue,
      reason: 'Activity filter choices must keep 48dp touch targets.',
    );

    await tester.tap(find.descendant(of: surface, matching: find.text('All')));
    await tester.pumpAndSettle();
    final optionsTrigger = find.byKey(const ValueKey('activity-options-menu'));
    expect(
      tester.getSize(optionsTrigger),
      const Size(Grid.xl, Grid.xl),
      reason: 'Activity options must retain a 48dp touch target.',
    );
    await tester.tap(optionsTrigger);
    await tester.pump();

    final optionsSurface = find.byKey(
      const ValueKey('activity-options-popover'),
    );
    final optionsMaterial = tester.widget<Material>(optionsSurface);
    final optionsShape = optionsMaterial.shape! as RoundedRectangleBorder;
    expect(tester.getSize(optionsSurface).width, 216);
    expect(optionsShape.borderRadius, BorderRadius.circular(Radii.popover));
    expect(optionsMaterial.elevation, appPopoverElevation);
    expect(
      tester
          .widget<ScaleTransition>(
            find.byKey(const ValueKey('activity-popover-scale')),
          )
          .alignment,
      Alignment.topRight,
    );
  });

  testWidgets('rows lead with sender, contextual label, and preview', (
    tester,
  ) async {
    await tester.pumpWidget(await buildTestable());
    await tester.pumpAndSettle();

    // Sender names resolved from the user cache.
    expect(find.text('Alice'), findsOneWidget);
    expect(find.text('alice@example.com'), findsOneWidget);
    expect(find.text('Bob'), findsOneWidget);
    expect(find.text('Scout'), findsOneWidget);

    // Contextual labels + channel chips.
    expect(find.text('Mentioned in'), findsOneWidget);
    expect(find.text('Thread in'), findsOneWidget);
    expect(find.text('#general'), findsNWidgets(2)); // mention + agent
    expect(find.text('#engineering'), findsOneWidget);

    // Context labels and channel pills share the compact Activity style.
    final contextLabel = tester.widget<Text>(find.text('Mentioned in'));
    final channelLabel = tester.widgetList<Text>(find.text('#general')).first;
    expect(contextLabel.style?.fontSize, activityContextTextStyle.fontSize);
    expect(contextLabel.style?.fontWeight, activityContextTextStyle.fontWeight);
    expect(contextLabel.style?.height, activityContextTextStyle.height);
    expect(channelLabel.style?.fontSize, activityContextTextStyle.fontSize);
    expect(channelLabel.style?.fontWeight, activityContextTextStyle.fontWeight);
    expect(channelLabel.style?.height, activityContextTextStyle.height);

    // Message previews.
    expect(find.textContaining('Hey check this out'), findsOneWidget);
    expect(find.textContaining('Deployed the fix'), findsOneWidget);

    // Activity rows use their conversation-oriented scale.
    final senderText = tester.widget<Text>(find.text('Alice'));
    final theme = Theme.of(tester.element(find.text('Alice')));
    expect(senderText.style?.fontSize, activityUsernameTextStyle.fontSize);
    expect(senderText.style?.fontWeight, activityUsernameTextStyle.fontWeight);
    expect(senderText.style?.height, activityUsernameTextStyle.height);
    expect(senderText.style?.color, theme.colorScheme.onSurface);
    final usernameText = tester.widget<Text>(
      find.byKey(const ValueKey('activity-username-m1')),
    );
    final timestampText = tester.widget<Text>(
      find.byKey(const ValueKey('activity-timestamp-m1')),
    );
    expect(usernameText.style?.fontSize, messageMetadataTextStyle.fontSize);
    expect(usernameText.style?.fontWeight, FontWeight.w400);
    expect(usernameText.style?.height, messageMetadataTextStyle.height);
    expect(timestampText.style?.fontSize, messageMetadataTextStyle.fontSize);
    expect(timestampText.style?.fontWeight, FontWeight.w400);

    final avatars = tester.widgetList<AvatarImage>(find.byType(AvatarImage));
    expect(avatars, isNotEmpty);
    expect(
      avatars.every((avatar) => avatar.radius == activityAvatarSize / 2),
      isTrue,
    );

    final previews = tester.widgetList<MessageContent>(
      find.byType(MessageContent),
    );
    expect(previews, isNotEmpty);
    expect(
      previews.every(
        (preview) =>
            preview.baseStyle?.fontSize == activityPreviewTextStyle.fontSize &&
            preview.baseStyle?.fontWeight ==
                activityPreviewTextStyle.fontWeight &&
            preview.baseStyle?.height == activityPreviewTextStyle.height &&
            preview.baseStyle?.letterSpacing ==
                activityPreviewTextStyle.letterSpacing &&
            preview.baseStyle?.color == theme.colorScheme.onSurface,
      ),
      isTrue,
    );
  });

  testWidgets('multiple top-level messages in one DM render one row', (
    tester,
  ) async {
    final dmChannel = Channel(
      id: 'dm1',
      name: 'dm',
      channelType: 'dm',
      visibility: 'private',
      description: '',
      createdBy: 'x',
      createdAt: DateTime(2025),
      memberCount: 2,
      isMember: true,
      participants: const ['Alice'],
    );
    FeedItem dmMessage(String id, int age) => FeedItem(
      id: id,
      kind: 9,
      pubkey: 'alice_pk',
      content: 'dm body $id',
      createdAt: now - age,
      channelId: 'dm1',
      channelName: '',
      tags: const [],
      category: 'activity',
    );

    await tester.pumpWidget(
      await buildTestable(
        feed: HomeFeedResponse(
          mentions: const [],
          needsAction: const [],
          activity: [dmMessage('dm-a', 300), dmMessage('dm-b', 200)],
          agentActivity: const [],
        ),
        channels: [...testChannels, dmChannel],
      ),
    );
    await tester.pumpAndSettle();

    // One conversation row for the DM, represented by the latest message.
    expect(find.byKey(const ValueKey('inbox-row-dm-b')), findsOneWidget);
    expect(find.byKey(const ValueKey('inbox-row-dm-a')), findsNothing);
    expect(find.textContaining('dm body dm-b'), findsOneWidget);
    expect(find.textContaining('dm body dm-a'), findsNothing);
  });

  testWidgets('unread rows show a dot; read rows do not', (tester) async {
    await tester.pumpWidget(
      await buildTestable(
        // ch1 fully read; ch2 (thread) unread.
        readContexts: {'ch1': now},
      ),
    );
    await tester.pumpAndSettle();

    expect(find.byKey(const ValueKey('inbox-unread-dot-m1')), findsNothing);
    expect(find.byKey(const ValueKey('inbox-unread-dot-ag1')), findsNothing);
    expect(find.byKey(const ValueKey('inbox-unread-dot-a1')), findsOneWidget);
  });

  testWidgets('New boundary separates unread rows from read rows', (
    tester,
  ) async {
    await tester.pumpWidget(
      await buildTestable(
        // Mention (2m ago) and agent (1m ago) unread; thread (1h ago) read.
        readContexts: {'thread:root1': now, 'ch2': now},
      ),
    );
    await tester.pumpAndSettle();

    expect(find.text('New'), findsOneWidget);
  });

  testWidgets('filter menu switches sources and shows empty states', (
    tester,
  ) async {
    await tester.pumpWidget(await buildTestable());
    await tester.pumpAndSettle();

    await tester.tap(find.byKey(const ValueKey('activity-filter-menu')));
    await tester.pumpAndSettle();
    await tester.tap(find.text('Mentions'));
    await tester.pumpAndSettle();

    expect(find.textContaining('Hey check this out'), findsOneWidget);
    expect(find.textContaining('Deployed the fix'), findsNothing);
    expect(find.textContaining('Job completed successfully'), findsNothing);

    await tester.tap(find.byKey(const ValueKey('activity-filter-menu')));
    await tester.pumpAndSettle();
    await tester.tap(find.text('Needs Action'));
    await tester.pumpAndSettle();

    expect(find.text('Nothing needs your action'), findsOneWidget);
  });

  testWidgets('filter menu supports accessibility text scaling', (
    tester,
  ) async {
    await tester.pumpWidget(
      await buildTestable(textScaler: const TextScaler.linear(3)),
    );
    await tester.pumpAndSettle();

    await tester.tap(find.byKey(const ValueKey('activity-filter-menu')));
    await tester.pumpAndSettle();

    expect(
      find.byKey(const ValueKey('activity-filter-popover')),
      findsOneWidget,
    );
    expect(tester.takeException(), isNull);
  });

  testWidgets('opens a thread mention at the referenced message', (
    tester,
  ) async {
    final threadMention = FeedItem(
      id: 'reply-event',
      kind: 9,
      pubkey: 'alice_pk',
      content: 'Reply in a thread',
      createdAt: DateTime.now().millisecondsSinceEpoch ~/ 1000,
      channelId: 'ch1',
      channelName: 'general',
      tags: const [
        ['e', 'thread-root', '', 'root'],
        ['e', 'parent-reply', '', 'reply'],
      ],
      category: 'mention',
    );
    final feed = HomeFeedResponse(
      mentions: [threadMention],
      needsAction: const [],
      activity: const [],
      agentActivity: const [],
    );

    await tester.pumpWidget(await buildTestable(feed: feed));
    await tester.pumpAndSettle();

    await tester.tap(find.byKey(const ValueKey('inbox-row-reply-event')));
    await tester.pumpAndSettle();

    final page = tester.widget<ChannelDetailPage>(
      find.byType(ChannelDetailPage),
    );
    expect(page.channel.id, 'ch1');
    expect(page.initialThreadRootId, 'parent-reply');
    expect(page.initialMessageId, 'reply-event');
  });

  testWidgets('thread filter matches grouped thread replies', (tester) async {
    await tester.pumpWidget(await buildTestable());
    await tester.pumpAndSettle();

    await tester.tap(find.byKey(const ValueKey('activity-filter-menu')));
    await tester.pumpAndSettle();
    await tester.tap(find.text('Threads'));
    await tester.pumpAndSettle();

    expect(find.textContaining('Deployed the fix'), findsOneWidget);
    expect(find.textContaining('Hey check this out'), findsNothing);
  });

  testWidgets('reminders filter shows the reminders empty surface', (
    tester,
  ) async {
    await tester.pumpWidget(await buildTestable());
    await tester.pumpAndSettle();

    await tester.tap(find.byKey(const ValueKey('activity-filter-menu')));
    await tester.pumpAndSettle();
    await tester.tap(find.text('Reminders'));
    await tester.pumpAndSettle();

    expect(find.text('No reminders'), findsOneWidget);
  });

  testWidgets('drafts filter shows the drafts empty surface', (tester) async {
    await tester.pumpWidget(await buildTestable());
    await tester.pumpAndSettle();

    await tester.tap(find.byKey(const ValueKey('activity-filter-menu')));
    await tester.pumpAndSettle();
    await tester.tap(find.text('Drafts'));
    await tester.pumpAndSettle();

    expect(find.text('No drafts'), findsOneWidget);
  });

  testWidgets('unread-only toggle hides read rows', (tester) async {
    await tester.pumpWidget(await buildTestable(readContexts: {'ch1': now}));
    await tester.pumpAndSettle();

    await tester.tap(find.byKey(const ValueKey('activity-options-menu')));
    await tester.pumpAndSettle();
    await tester.tap(find.text('Show unread'));
    await tester.pumpAndSettle();

    // Only the unread thread row remains.
    expect(find.textContaining('Deployed the fix'), findsOneWidget);
    expect(find.textContaining('Hey check this out'), findsNothing);
    expect(find.textContaining('Job completed successfully'), findsNothing);
  });

  testWidgets('long-press mark unread reopens a read row', (tester) async {
    await tester.pumpWidget(
      await buildTestable(
        readContexts: {'ch1': now, 'ch2': now, 'thread:root1': now},
      ),
    );
    await tester.pumpAndSettle();

    expect(find.byKey(const ValueKey('inbox-unread-dot-m1')), findsNothing);

    await tester.longPress(find.byKey(const ValueKey('inbox-row-m1')));
    await tester.pumpAndSettle();
    await tester.tap(
      find.descendant(
        of: find.byType(BottomSheet),
        matching: find.text('Mark unread'),
      ),
    );
    await tester.pumpAndSettle();

    expect(find.byKey(const ValueKey('inbox-unread-dot-m1')), findsOneWidget);
  });

  testWidgets('swiping an inbox row reveals and runs its row actions', (
    tester,
  ) async {
    await tester.pumpWidget(
      await buildTestable(
        readContexts: {'ch1': now, 'ch2': now, 'thread:root1': now},
      ),
    );
    await tester.pumpAndSettle();

    final row = find.byKey(const ValueKey('inbox-row-m1'));
    final originalLeft = tester.getTopLeft(row).dx;
    await tester.drag(row, const Offset(-200, 0));
    await tester.pumpAndSettle();

    expect(tester.getTopLeft(row).dx, lessThan(originalLeft));
    expect(find.byKey(const ValueKey('inbox-swipe-read-m1')), findsOneWidget);
    expect(find.byKey(const ValueKey('inbox-swipe-open-m1')), findsNothing);
    expect(
      find.byKey(const ValueKey('inbox-swipe-background-m1')),
      findsOneWidget,
    );
    expect(
      tester
          .getSize(find.byKey(const ValueKey('inbox-swipe-background-m1')))
          .height,
      tester.getSize(row).height,
    );
    final markUnreadAction = find.byKey(const ValueKey('inbox-swipe-read-m1'));
    final markUnreadMaterial = tester.widget<Material>(
      find.descendant(of: markUnreadAction, matching: find.byType(Material)),
    );
    expect(markUnreadMaterial.color, tester.element(row).colors.primary);
    expect(markUnreadMaterial.borderRadius, BorderRadius.circular(Radii.full));

    await tester.tap(find.byKey(const ValueKey('inbox-swipe-read-m1')));
    await tester.pumpAndSettle();

    expect(find.byKey(const ValueKey('inbox-unread-dot-m1')), findsOneWidget);

    await tester.drag(row, const Offset(-200, 0));
    await tester.pumpAndSettle();
    final markReadAction = find.byKey(const ValueKey('inbox-swipe-read-m1'));
    final markReadMaterial = tester.widget<Material>(
      find.descendant(of: markReadAction, matching: find.byType(Material)),
    );
    expect(markReadMaterial.color, tester.element(row).appColors.success);
    expect(markReadMaterial.color, isNot(markUnreadMaterial.color));
  });

  testWidgets('swipe action reveals its label with one threshold haptic', (
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

    await tester.pumpWidget(
      await buildTestable(
        readContexts: {'ch1': now, 'ch2': now, 'thread:root1': now},
      ),
    );
    await tester.pumpAndSettle();

    final row = find.byKey(const ValueKey('inbox-row-m1'));
    final gesture = await tester.startGesture(tester.getCenter(row));
    for (var step = 1; step <= 7; step++) {
      await gesture.moveBy(
        const Offset(-10, 0),
        timeStamp: Duration(milliseconds: step * 50),
      );
      await tester.pump();
    }

    final action = find.byKey(const ValueKey('inbox-swipe-read-m1'));
    expect(action, findsOneWidget);
    expect(
      find.descendant(of: action, matching: find.byIcon(LucideIcons.mail)),
      findsOneWidget,
    );
    expect(find.text('Mark unread'), findsNothing);
    expect(hapticCalls, isEmpty);

    for (var step = 8; step <= 10; step++) {
      await gesture.moveBy(
        const Offset(-10, 0),
        timeStamp: Duration(milliseconds: step * 50),
      );
      await tester.pump();
    }
    expect(find.text('Mark unread'), findsOneWidget);
    expect(hapticCalls, hasLength(1));
    expect(hapticCalls.single.arguments, 'HapticFeedbackType.selectionClick');

    await gesture.moveBy(
      const Offset(20, 0),
      timeStamp: const Duration(milliseconds: 550),
    );
    await tester.pump();
    await gesture.moveBy(
      const Offset(-20, 0),
      timeStamp: const Duration(milliseconds: 600),
    );
    await tester.pump();
    expect(hapticCalls, hasLength(1));

    await gesture.up();
    await tester.pumpAndSettle();
  });

  testWidgets('commits the inbox read-state action past the swipe threshold', (
    tester,
  ) async {
    await tester.pumpWidget(
      await buildTestable(
        readContexts: {'ch1': now, 'ch2': now, 'thread:root1': now},
      ),
    );
    await tester.pumpAndSettle();

    final row = find.byKey(const ValueKey('inbox-row-m1'));
    await tester.drag(row, const Offset(-500, 0));
    await tester.pumpAndSettle();

    expect(find.byKey(const ValueKey('inbox-unread-dot-m1')), findsOneWidget);
    expect(
      find.byKey(const ValueKey('inbox-swipe-background-m1')),
      findsNothing,
    );
  });

  testWidgets('falls back to short pubkey when user not cached', (
    tester,
  ) async {
    await tester.pumpWidget(await buildTestable(users: const {}));
    await tester.pumpAndSettle();

    // Sender label falls back to the (short) pubkey.
    expect(find.text('alice_pk'), findsOneWidget);
    expect(find.text('Alice'), findsNothing);
  });
}

class _FakeActivityNotifier extends ActivityNotifier {
  final HomeFeedResponse _feed;
  _FakeActivityNotifier(this._feed);

  @override
  Future<HomeFeedResponse> build() async => _feed;
}

class _PendingActivityNotifier extends ActivityNotifier {
  @override
  Future<HomeFeedResponse> build() => Completer<HomeFeedResponse>().future;
}

class _ErrorActivityNotifier extends ActivityNotifier {
  @override
  Future<HomeFeedResponse> build() => Future.error('Connection refused');
}

class _FakeChannelsNotifier extends ChannelsNotifier {
  final List<Channel> _channels;
  _FakeChannelsNotifier(this._channels);

  @override
  Future<List<Channel>> build() async => _channels;
}

class _FakeUserCacheNotifier extends UserCacheNotifier {
  final Map<String, UserProfile> _users;
  _FakeUserCacheNotifier(this._users);

  @override
  Map<String, UserProfile> build() => _users;
}

class _FakeReadStateNotifier extends ReadStateNotifier {
  final Map<String, int> _contexts;
  _FakeReadStateNotifier(this._contexts);

  @override
  ReadStateState build() => ReadStateState(
    isReady: true,
    pubkey: 'me_pk',
    contexts: Map.unmodifiable(_contexts),
    version: 1,
  );

  @override
  void markContextRead(
    String contextId,
    int unixTimestamp, {
    bool clearForcedMessages = false,
  }) {
    state = state.copyWithContext(contextId, unixTimestamp);
  }
}

class _FakeRemindersNotifier extends RemindersNotifier {
  final List<Reminder> _reminders;
  _FakeRemindersNotifier(this._reminders);

  @override
  Future<List<Reminder>> build() async => _reminders;
}
