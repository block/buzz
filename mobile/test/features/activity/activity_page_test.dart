import 'dart:async';
import 'dart:io';
import 'dart:ui' as ui;
import 'package:flutter/rendering.dart';
import 'package:buzz/shared/widgets/ios_message_create_button.dart';
import 'package:buzz/shared/widgets/ios_glass_navigation_button.dart';

import 'package:flutter/foundation.dart';
import 'package:buzz/features/activity/activity_page.dart';
import 'package:buzz/features/activity/activity_provider.dart';
import 'package:buzz/features/activity/compose_drafts_provider.dart';
import 'package:buzz/features/activity/feed_item.dart';
import 'package:buzz/features/activity/reminders_provider.dart';
import 'package:buzz/features/channels/channel.dart';
import 'package:buzz/features/channels/channel_detail_page.dart';
import 'package:buzz/features/channels/channel_management_provider.dart';
import 'package:buzz/features/channels/message_content.dart';
import 'package:buzz/features/channels/channels_provider.dart';
import 'package:buzz/features/channels/thread_follows/thread_follows_provider.dart';
import 'package:buzz/shared/identity_names/identity_names.dart';
import 'package:buzz/shared/mentions/agent_identity_provider.dart';
import 'package:buzz/shared/read_state/read_state_format.dart';
import 'package:buzz/shared/read_state/read_state_provider.dart';
import 'package:buzz/shared/relay/relay.dart';
import 'package:buzz/shared/reminders/reminder_service.dart' as reminder;
import 'package:buzz/shared/profile/user_cache_provider.dart';
import 'package:buzz/shared/profile/user_profile.dart';
import 'package:buzz/shared/theme/theme.dart';
import 'package:buzz/shared/widgets/anchored_popover_menu.dart';
import 'package:buzz/shared/widgets/frosted_app_bar.dart';
import 'package:buzz/shared/widgets/avatar_image.dart';
import 'package:flutter/material.dart';
import 'package:flutter/services.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';
import 'package:hooks_riverpod/misc.dart';
import 'package:buzz/shared/theme/buzz_icons.dart';
import 'package:nostr/nostr.dart' as nostr;
import 'package:shared_preferences/shared_preferences.dart';

import '../../helpers/thread_ids.dart';

part 'activity_page_test/tabs_journey.dart';

void main() {
  final now = DateTime.now().millisecondsSinceEpoch ~/ 1000;

  final testMention = FeedItem(
    id: 'm1',
    kind: 9,
    pubkey: 'a11ce00000000000000000000000000000000000000000000000000000000000',
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
      ['e', threadIdRoot1, '', 'root'],
      ['e', threadIdRoot1, '', 'reply'],
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
    'a11ce00000000000000000000000000000000000000000000000000000000000':
        const UserProfile(
          pubkey:
              'a11ce00000000000000000000000000000000000000000000000000000000000',
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
    List<ComposeDraft> drafts = const [],
    List<Reminder> reminders = const [],
    RemindersNotifier Function()? remindersNotifier,
    Set<String> knownAgentPubkeys = const {},
    List<Override> overrides = const [],
    UserCacheNotifier Function()? userCache,
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
          userCache ?? () => _FakeUserCacheNotifier(users ?? testUsers),
        ),
        knownAgentPubkeysProvider.overrideWithValue(knownAgentPubkeys),
        readStateProvider.overrideWith(
          () => _FakeReadStateNotifier(readContexts),
        ),
        composeDraftsProvider.overrideWith(
          () => _FakeComposeDraftsNotifier(drafts),
        ),
        remindersProvider.overrideWith(
          remindersNotifier ?? () => _FakeRemindersNotifier(reminders),
        ),
        ...overrides,
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

  _activityTabJourneys(
    ({Set<String> knownAgentPubkeys = const {}, TextScaler? textScaler}) =>
        buildTestable(
          knownAgentPubkeys: knownAgentPubkeys,
          textScaler: textScaler,
        ),
  );

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

  testWidgets(
    'Activity keeps its header above the clipped feed at large text',
    (tester) async {
      await tester.pumpWidget(
        await buildTestable(textScaler: const TextScaler.linear(2)),
      );
      await tester.pumpAndSettle();
      expect(find.byTooltip('Close activity'), findsOneWidget);
      expect(find.byTooltip('Back'), findsNothing);
      expect(find.byType(FrostedAppBar), findsNothing);
      final filters = find.byKey(const ValueKey('activity-filters'));
      final viewport = find.byKey(const ValueKey('activity-content-viewport'));
      final cutoff = tester.getBottomLeft(filters).dy;
      expect(tester.getTopLeft(viewport).dy, cutoff);
      expect(
        tester.getBottomLeft(find.text('Activity')).dy,
        lessThanOrEqualTo(tester.getTopLeft(filters).dy),
      );
      await tester.drag(find.byType(CustomScrollView), const Offset(0, -300));
      await tester.pumpAndSettle();
      expect(tester.getTopLeft(viewport).dy, cutoff);
      expect(tester.getBottomLeft(filters).dy, cutoff);
      expect(tester.takeException(), isNull);
    },
  );

  testWidgets('keeps footer clearance inside the scrollable content', (
    tester,
  ) async {
    await tester.pumpWidget(
      await buildTestable(mediaPadding: const EdgeInsets.only(bottom: 88)),
    );
    await tester.pumpAndSettle();

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

    final filterTrigger = find.byKey(const ValueKey('activity-sender-menu'));
    expect(
      tester.getSize(filterTrigger).height,
      greaterThanOrEqualTo(Grid.xl),
      reason: 'The Activity filter trigger must keep a 48dp touch target.',
    );

    await tester.ensureVisible(filterTrigger);
    await tester.pumpAndSettle();
    await tester.tap(filterTrigger);
    await tester.pump();

    final surface = find.byKey(const ValueKey('activity-sender-popover'));
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

    final items = tester.widgetList<PopupMenuItem<String>>(
      find.byType(PopupMenuItem<String>),
    );
    expect(items, hasLength(3));
    expect(
      items.every((item) => item.height >= Grid.xl),
      isTrue,
      reason: 'Activity filter choices must keep 48dp touch targets.',
    );

    await tester.tap(
      find.descendant(of: surface, matching: find.text('Anyone')),
    );
    await tester.pumpAndSettle();
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
    expect(
      find.text('#general'),
      findsNWidgets(2),
    ); // mention and agent activity
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
    expect(timestampText.style?.fontSize, activityTimestampTextStyle.fontSize);
    expect(timestampText.style?.fontWeight, FontWeight.w400);
    expect(
      timestampText.style?.fontSize,
      lessThan(usernameText.style!.fontSize!),
    );

    final avatars = tester.widgetList<AvatarImage>(find.byType(AvatarImage));
    expect(avatars, isNotEmpty);
    expect(
      avatars.every((avatar) => avatar.radius == activityAvatarSize / 2),
      isTrue,
    );

    final previews = tester.widgetList<MessagePreview>(
      find.byType(MessagePreview),
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

  testWidgets('blank cached sender names fall back to the compact npub', (
    tester,
  ) async {
    // Relay profiles can cache blank display names (empty and
    // whitespace-only) unchanged, so the sender must resolve through the
    // shared nonblank-name label contract: the row shows the compact npub
    // of the a11ce key instead of a blank author label. Binds the production
    // seam — the sender resolves through the user cache exactly as the live
    // page does. Keyed remounts keep each ProviderScope (and its user-cache
    // override) fresh between scenarios, so each iteration actually
    // consumes its own blank-name fixture.
    const sender =
        'a11ce00000000000000000000000000000000000000000000000000000000000';
    for (final blankName in const ['', '   ']) {
      await tester.pumpWidget(
        KeyedSubtree(
          key: ValueKey('blank-sender-${blankName.length}'),
          child: await buildTestable(
            users: {
              ...testUsers,
              sender: UserProfile(pubkey: sender, displayName: blankName),
            },
          ),
        ),
      );
      await tester.pumpAndSettle();

      expect(find.text(blankName), findsNothing);
      expect(
        find.descendant(
          of: find.byKey(const ValueKey('inbox-row-m1')),
          matching: find.text('npub15yw…ccpw'),
        ),
        findsOneWidget,
      );
    }
  });

  testWidgets('same-name senders get their channel\'s contextual label', (
    tester,
  ) async {
    // Another ch1 member shares the sender's name, so the row must use the
    // channel's disambiguated label rather than the bare profile name.
    const sender =
        'a11ce00000000000000000000000000000000000000000000000000000000000';
    const twin =
        'b0b0000000000000000000000000000000000000000000000000000000000000';
    final members = [
      for (final pubkey in const [sender, twin])
        ChannelMember(pubkey: pubkey, role: 'member', joinedAt: DateTime(2025)),
    ];
    final users = {
      ...testUsers,
      sender: const UserProfile(pubkey: sender, displayName: 'Scout'),
      twin: const UserProfile(pubkey: twin, displayName: 'Scout'),
    };
    await tester.pumpWidget(
      await buildTestable(
        users: users,
        overrides: [
          channelMembersProvider('ch1').overrideWith((ref) async => members),
        ],
      ),
    );
    await tester.pumpAndSettle();

    final expected = IdentityNameSources(
      profiles: users,
    ).scope(const [sender, twin]).labelFor(sender);
    expect(expected, isNot('Scout'));
    expect(
      find.descendant(
        of: find.byKey(const ValueKey('inbox-row-m1')),
        matching: find.textContaining(expected),
      ),
      findsWidgets,
    );
  });

  testWidgets('directory-known Activity authors use agent avatars', (
    tester,
  ) async {
    await tester.pumpWidget(
      await buildTestable(knownAgentPubkeys: const {'bob_pk'}),
    );
    await tester.pumpAndSettle();

    final agentRow = find.byKey(const ValueKey('inbox-row-a1'));
    final agentAvatar = tester.widget<AvatarImage>(
      find.descendant(of: agentRow, matching: find.byType(AvatarImage)),
    );
    final humanRow = find.byKey(const ValueKey('inbox-row-m1'));
    final humanAvatar = tester.widget<AvatarImage>(
      find.descendant(of: humanRow, matching: find.byType(AvatarImage)),
    );
    expect(agentAvatar.isAgent, isTrue);
    expect(humanAvatar.isAgent, isFalse);
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
      pubkey:
          'a11ce00000000000000000000000000000000000000000000000000000000000',
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

  testWidgets('marking a DM row read covers replies under every thread root', (
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
    FeedItem reply(String id, String root, int age) => FeedItem(
      id: id,
      kind: 9,
      pubkey:
          'a11ce00000000000000000000000000000000000000000000000000000000000',
      content: 'dm reply $id',
      createdAt: now - age,
      channelId: 'dm1',
      channelName: '',
      tags: [
        ['e', root, '', 'root'],
        ['e', root, '', 'reply'],
      ],
      category: 'activity',
    );
    const rootA =
        'aaaa000000000000000000000000000000000000000000000000000000000001';
    const rootB =
        'bbbb000000000000000000000000000000000000000000000000000000000002';

    await tester.pumpWidget(
      await buildTestable(
        feed: HomeFeedResponse(
          mentions: const [],
          needsAction: const [],
          activity: [reply('dm-r1', rootA, 300), reply('dm-r2', rootB, 200)],
          agentActivity: const [],
        ),
        channels: [...testChannels, dmChannel],
      ),
    );
    await tester.pumpAndSettle();
    expect(
      find.byKey(const ValueKey('inbox-unread-dot-dm-r2')),
      findsOneWidget,
    );

    await tester.drag(
      find.byKey(const ValueKey('inbox-row-dm-r2')),
      const Offset(-500, 0),
    );
    await tester.pumpAndSettle();

    expect(find.byKey(const ValueKey('inbox-unread-dot-dm-r2')), findsNothing);
  });

  testWidgets(
    'sender filter keeps a person\'s message under a later agent reply',
    (tester) async {
      FeedItem threadEvent(String id, String pubkey, String body, int age) =>
          FeedItem(
            id: id,
            kind: 9,
            pubkey: pubkey,
            content: body,
            createdAt: now - age,
            channelId: 'ch1',
            channelName: 'general',
            tags: const [
              ['e', threadIdRoot1, '', 'root'],
              ['e', threadIdRoot1, '', 'reply'],
            ],
            category: 'mention',
          );

      await tester.pumpWidget(
        await buildTestable(
          knownAgentPubkeys: const {'agent_pk'},
          feed: HomeFeedResponse(
            mentions: [
              threadEvent('p1', 'bob_pk', 'person asks', 300),
              threadEvent('g1', 'agent_pk', 'agent answers', 200),
            ],
            needsAction: const [],
            activity: const [],
            agentActivity: const [],
          ),
        ),
      );
      await tester.pumpAndSettle();
      expect(find.textContaining('agent answers'), findsOneWidget);

      await tester.tap(find.byKey(const ValueKey('activity-sender-menu')));
      await tester.pumpAndSettle();
      await tester.tap(find.text('People').last);
      await tester.pumpAndSettle();

      expect(find.textContaining('person asks'), findsOneWidget);
      expect(find.textContaining('agent answers'), findsNothing);
    },
  );

  testWidgets('Agents filter counts channel bot members as agents', (
    tester,
  ) async {
    FeedItem event(String id, String pubkey, String body, int age) => FeedItem(
      id: id,
      kind: 9,
      pubkey: pubkey,
      content: body,
      createdAt: now - age,
      channelId: 'ch1',
      channelName: 'general',
      tags: const [],
      category: 'mention',
    );

    await tester.pumpWidget(
      await buildTestable(
        feed: HomeFeedResponse(
          mentions: [
            event('b1', 'bot_pk', 'bot reports', 300),
            event('p1', 'bob_pk', 'person asks', 200),
          ],
          needsAction: const [],
          activity: const [],
          agentActivity: const [],
        ),
        overrides: [
          channelBotPubkeysProvider(
            'ch1',
          ).overrideWith((ref) async => {'bot_pk'}),
        ],
      ),
    );
    await tester.pumpAndSettle();

    await tester.tap(find.byKey(const ValueKey('activity-sender-menu')));
    await tester.pumpAndSettle();
    await tester.tap(find.text('Agents').last);
    await tester.pumpAndSettle();

    expect(find.textContaining('bot reports'), findsOneWidget);
    expect(find.textContaining('person asks'), findsNothing);
  });

  testWidgets('Threads filter shows a DM reply, not a later DM message', (
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
    FeedItem dmEvent(
      String id,
      String body,
      int age,
      List<List<String>> tags,
    ) => FeedItem(
      id: id,
      kind: 9,
      pubkey:
          'a11ce00000000000000000000000000000000000000000000000000000000000',
      content: body,
      createdAt: now - age,
      channelId: 'dm1',
      channelName: '',
      tags: tags,
      category: 'activity',
    );

    await tester.pumpWidget(
      await buildTestable(
        feed: HomeFeedResponse(
          mentions: const [],
          needsAction: const [],
          activity: [
            dmEvent('dm-reply', 'reply in thread', 300, const [
              ['e', threadIdRoot1, '', 'root'],
              ['e', threadIdRoot1, '', 'reply'],
            ]),
            dmEvent('dm-top', 'later top level', 200, const []),
          ],
          agentActivity: const [],
        ),
        channels: [...testChannels, dmChannel],
      ),
    );
    await tester.pumpAndSettle();
    await _chooseActivityType(tester, 'Threads');

    expect(find.textContaining('reply in thread'), findsOneWidget);
    expect(find.textContaining('later top level'), findsNothing);
  });

  testWidgets(
    'reading a DM reply under Threads leaves other DM messages unread',
    (tester) async {
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
      FeedItem dmEvent(String id, int age, List<List<String>> tags) => FeedItem(
        id: id,
        kind: 9,
        pubkey:
            'a11ce00000000000000000000000000000000000000000000000000000000000',
        content: 'dm body $id',
        createdAt: now - age,
        channelId: 'dm1',
        channelName: '',
        tags: tags,
        category: 'activity',
      );

      await tester.pumpWidget(
        await buildTestable(
          feed: HomeFeedResponse(
            mentions: const [],
            needsAction: const [],
            activity: [
              dmEvent('dm-top', 300, const []),
              dmEvent('dm-reply', 200, const [
                ['e', threadIdRoot1, '', 'root'],
                ['e', threadIdRoot1, '', 'reply'],
              ]),
            ],
            agentActivity: const [],
          ),
          channels: [...testChannels, dmChannel],
        ),
      );
      await tester.pumpAndSettle();
      await _chooseActivityType(tester, 'Threads');
      await tester.drag(
        find.byKey(const ValueKey('inbox-row-dm-reply')),
        const Offset(-500, 0),
      );
      await tester.pumpAndSettle();
      expect(
        find.byKey(const ValueKey('inbox-unread-dot-dm-reply')),
        findsNothing,
      );

      await _chooseActivityType(tester, 'All');
      expect(
        find.byKey(const ValueKey('inbox-unread-dot-dm-reply')),
        findsOneWidget,
      );
    },
  );

  testWidgets(
    'reading a thread row under People leaves hidden agent replies unread',
    (tester) async {
      FeedItem threadEvent(String id, String pubkey, int age) => FeedItem(
        id: id,
        kind: 9,
        pubkey: pubkey,
        content: 'body $id',
        createdAt: now - age,
        channelId: 'ch1',
        channelName: 'general',
        tags: const [
          ['e', threadIdRoot1, '', 'root'],
          ['e', threadIdRoot1, '', 'reply'],
        ],
        category: 'mention',
      );

      await tester.pumpWidget(
        await buildTestable(
          knownAgentPubkeys: const {'agent_pk'},
          feed: HomeFeedResponse(
            mentions: [
              threadEvent('g1', 'agent_pk', 300),
              threadEvent('p1', 'bob_pk', 200),
            ],
            needsAction: const [],
            activity: const [],
            agentActivity: const [],
          ),
        ),
      );
      await tester.pumpAndSettle();

      Future<void> chooseSender(String label) async {
        await tester.tap(find.byKey(const ValueKey('activity-sender-menu')));
        await tester.pumpAndSettle();
        await tester.tap(find.text(label).last);
        await tester.pumpAndSettle();
      }

      await chooseSender('People');
      await tester.drag(
        find.byKey(const ValueKey('inbox-row-p1')),
        const Offset(-500, 0),
      );
      await tester.pumpAndSettle();
      expect(find.byKey(const ValueKey('inbox-unread-dot-p1')), findsNothing);

      await chooseSender('Anyone');
      expect(find.byKey(const ValueKey('inbox-unread-dot-p1')), findsOneWidget);
      int? threadMarker() => ProviderScope.containerOf(
        tester.element(find.byType(ActivityPage)),
      ).read(readStateProvider).contexts[threadContextKey(threadIdRoot1)];
      // The synced marker stops short of the hidden agent reply.
      expect(threadMarker(), now - 301);

      await chooseSender('Agents');
      await tester.drag(
        find.byKey(const ValueKey('inbox-row-g1')),
        const Offset(-500, 0),
      );
      await tester.pumpAndSettle();
      // With the agent reply read too, the synced marker covers both.
      expect(threadMarker(), now - 200);
      await chooseSender('Anyone');
      expect(find.byKey(const ValueKey('inbox-unread-dot-p1')), findsNothing);
    },
  );

  testWidgets('sender filter preloads authors hidden under newer rows', (
    tester,
  ) async {
    final cache = _RecordingUserCacheNotifier(testUsers);
    FeedItem threadEvent(String id, String pubkey, int age) => FeedItem(
      id: id,
      kind: 9,
      pubkey: pubkey,
      content: 'body $id',
      createdAt: now - age,
      channelId: 'ch1',
      channelName: 'general',
      tags: const [
        ['e', threadIdRoot1, '', 'root'],
        ['e', threadIdRoot1, '', 'reply'],
      ],
      category: 'mention',
    );

    await tester.pumpWidget(
      await buildTestable(
        feed: HomeFeedResponse(
          mentions: [
            threadEvent('g1', 'owned_agent_pk', 300),
            threadEvent('p1', 'bob_pk', 200),
          ],
          needsAction: const [],
          activity: const [],
          agentActivity: const [],
        ),
        userCache: () => cache,
      ),
    );
    await tester.pumpAndSettle();
    expect(cache.preloaded, isNot(contains('owned_agent_pk')));

    await tester.tap(find.byKey(const ValueKey('activity-sender-menu')));
    await tester.pumpAndSettle();
    await tester.tap(find.text('Agents').last);
    await tester.pumpAndSettle();

    expect(cache.preloaded, contains('owned_agent_pk'));
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

  testWidgets(
    'chronological Activity does not imply a contiguous unread boundary',
    (tester) async {
      await tester.pumpWidget(
        await buildTestable(
          // Mention (2m ago) and agent (1m ago) unread; thread (1h ago) read.
          readContexts: {'thread:$threadIdRoot1': now, 'ch2': now},
        ),
      );
      await tester.pumpAndSettle();

      expect(find.text('New'), findsNothing);
    },
  );

  testWidgets('tabs switch sources and show empty states', (tester) async {
    await tester.pumpWidget(await buildTestable());
    await tester.pumpAndSettle();

    await _chooseActivityType(tester, 'Mentions');
    await tester.pumpAndSettle();

    expect(find.textContaining('Hey check this out'), findsOneWidget);
    expect(find.textContaining('Deployed the fix'), findsNothing);
    expect(find.textContaining('Job completed successfully'), findsNothing);

    await _chooseActivityType(tester, 'DMs');
    await tester.pumpAndSettle();

    expect(find.text('No direct messages yet'), findsOneWidget);
  });

  testWidgets('sender menu supports accessibility text scaling', (
    tester,
  ) async {
    await tester.pumpWidget(
      await buildTestable(textScaler: const TextScaler.linear(3)),
    );
    await tester.pumpAndSettle();

    await tester.ensureVisible(
      find.byKey(const ValueKey('activity-sender-menu')),
    );
    await tester.pumpAndSettle();
    await tester.tap(find.byKey(const ValueKey('activity-sender-menu')));
    await tester.pumpAndSettle();

    expect(
      find.byKey(const ValueKey('activity-sender-popover')),
      findsOneWidget,
    );
    expect(tester.takeException(), isNull);
  });

  testWidgets('opens a thread reply at the referenced message', (tester) async {
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
    // A DM reply must still open its thread: the DM timeline hides replies.
    for (final channelId in ['ch1', 'dm1']) {
      final threadMention = FeedItem(
        id: 'reply-event',
        kind: 9,
        pubkey:
            'a11ce00000000000000000000000000000000000000000000000000000000000',
        content: 'Reply in a thread',
        createdAt: DateTime.now().millisecondsSinceEpoch ~/ 1000,
        channelId: channelId,
        channelName: 'general',
        tags: const [
          ['e', threadIdThreadDashRoot, '', 'root'],
          ['e', threadIdParentDashReply, '', 'reply'],
        ],
        category: 'mention',
      );
      final feed = HomeFeedResponse(
        mentions: [threadMention],
        needsAction: const [],
        activity: const [],
        agentActivity: const [],
      );

      await tester.pumpWidget(
        await buildTestable(feed: feed, channels: [...testChannels, dmChannel]),
      );
      await tester.pumpAndSettle();

      await tester.tap(find.byKey(const ValueKey('inbox-row-reply-event')));
      await tester.pumpAndSettle();

      final page = tester.widget<ChannelDetailPage>(
        find.byType(ChannelDetailPage),
      );
      expect(page.channel.id, channelId);
      expect(page.initialThreadRootId, threadIdParentDashReply);
      expect(page.initialMessageId, 'reply-event');
      expect(
        page.initialThreadRouteBehavior,
        InitialThreadRouteBehavior.replaceCurrentRoute,
      );
      await tester.pumpWidget(const SizedBox.shrink());
      await tester.pumpAndSettle();
    }
  });

  testWidgets('thread filter matches grouped thread replies', (tester) async {
    await tester.pumpWidget(await buildTestable());
    await tester.pumpAndSettle();

    await _chooseActivityType(tester, 'Threads');
    await tester.pumpAndSettle();

    expect(find.textContaining('Deployed the fix'), findsOneWidget);
    expect(find.textContaining('Hey check this out'), findsNothing);
  });

  testWidgets('unread-only toggle hides read rows', (tester) async {
    await tester.pumpWidget(await buildTestable(readContexts: {'ch1': now}));
    await tester.pumpAndSettle();

    expect(find.byKey(const ValueKey('activity-options-menu')), findsNothing);
    expect(find.text('Mark all as read'), findsNothing);
    await tester.tap(find.byKey(const ValueKey('activity-unread-filter')));
    await tester.pumpAndSettle();

    // Only the unread thread row remains.
    expect(find.textContaining('Deployed the fix'), findsOneWidget);
    expect(find.textContaining('Hey check this out'), findsNothing);
    expect(find.textContaining('Job completed successfully'), findsNothing);
  });

  testWidgets('long-press mark unread reopens a read row', (tester) async {
    await tester.pumpWidget(
      await buildTestable(
        readContexts: {'ch1': now, 'ch2': now, 'thread:$threadIdRoot1': now},
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

  testWidgets('long-press follows a thread and sets a reminder', (
    tester,
  ) async {
    final reminders = _CapturingReminderService();
    final remindersNotifier = _FakeRemindersNotifier(const []);
    await tester.pumpWidget(
      await buildTestable(
        remindersNotifier: () => remindersNotifier,
        overrides: [
          reminder.reminderServiceProvider.overrideWithValue(reminders),
        ],
      ),
    );
    await tester.pumpAndSettle();
    final container = ProviderScope.containerOf(
      tester.element(find.byType(ActivityPage)),
    );
    Finder sheetText(String text) => find.descendant(
      of: find.byType(BottomSheet),
      matching: find.text(text),
    );

    // A thread row follows its root, and the menu then offers unfollow.
    await tester.longPress(find.byKey(const ValueKey('inbox-row-a1')));
    await tester.pumpAndSettle();
    await tester.tap(sheetText('Follow thread'));
    await tester.pumpAndSettle();
    expect(
      container.read(threadFollowsProvider).isFollowing(threadIdRoot1),
      isTrue,
    );

    await tester.longPress(find.byKey(const ValueKey('inbox-row-a1')));
    await tester.pumpAndSettle();
    expect(sheetText('Follow thread'), findsNothing);
    await tester.tap(sheetText('Unfollow thread'));
    await tester.pumpAndSettle();
    expect(
      container.read(threadFollowsProvider).isFollowing(threadIdRoot1),
      isFalse,
    );

    // A top-level row follows the message itself as a potential thread head.
    await tester.longPress(find.byKey(const ValueKey('inbox-row-m1')));
    await tester.pumpAndSettle();
    await tester.tap(sheetText('Follow thread'));
    await tester.pumpAndSettle();
    expect(container.read(threadFollowsProvider).isFollowing('m1'), isTrue);

    // Remind me opens the shared reminder sheet for the row's message.
    await tester.longPress(find.byKey(const ValueKey('inbox-row-m1')));
    await tester.pumpAndSettle();
    await tester.tap(sheetText('Remind me'));
    await tester.pumpAndSettle();
    expect(find.text('Remind me about this message'), findsOneWidget);
    await tester.tap(find.text('In 1 hour'));
    await tester.pumpAndSettle();

    final first = reminders.saved.single;
    expect(first.target.eventId, 'm1');
    expect(first.target.channelId, 'ch1');
    expect(first.target.preview, 'Hey check this out');
    expect(first.dTag, isNull);
    expect(find.text('Reminder set'), findsOneWidget);
    expect(remindersNotifier.refreshes, 1);
  });

  testWidgets('long-press reschedules an existing reminder instead of adding '
      'one', (tester) async {
    final reminders = _CapturingReminderService();
    final existing = Reminder(
      id: 'reminder-d1',
      notBefore: now + 600,
      status: 'pending',
      target: const ReminderTarget(
        eventId: 'm1',
        channelId: 'ch1',
        preview: 'Hey check this out',
        authorPubkey:
            'a11ce00000000000000000000000000000000000000000000000000000000000',
      ),
      note: 'Reply after standup',
      createdAt: now - 60,
      eventId: 'reminder-event-1',
    );
    await tester.pumpWidget(
      await buildTestable(
        reminders: [existing],
        overrides: [
          reminder.reminderServiceProvider.overrideWithValue(reminders),
        ],
      ),
    );
    await tester.pumpAndSettle();

    await tester.longPress(find.byKey(const ValueKey('inbox-row-m1')));
    await tester.pumpAndSettle();
    await tester.tap(find.text('Remind me'));
    await tester.pumpAndSettle();
    await tester.tap(find.text('In 1 hour'));
    await tester.pumpAndSettle();

    final saved = reminders.saved.single;
    expect(saved.dTag, 'reminder-d1');
    expect(saved.note, 'Reply after standup');
    expect(find.text('Reminder updated'), findsOneWidget);
  });

  testWidgets('long-press omits Follow in a Solo chat', (tester) async {
    await tester.pumpWidget(
      await buildTestable(
        channels: [
          Channel(
            id: 'ch1',
            name: 'Notes',
            channelType: 'stream',
            visibility: 'private',
            description: '',
            createdBy: 'x',
            createdAt: DateTime(2025),
            memberCount: 1,
            isMember: true,
            labels: const ['buzz.solo'],
          ),
          testChannels[1],
        ],
      ),
    );
    await tester.pumpAndSettle();

    await tester.longPress(find.byKey(const ValueKey('inbox-row-m1')));
    await tester.pumpAndSettle();
    expect(find.text('Open conversation'), findsOneWidget);
    expect(find.text('Follow thread'), findsNothing);
  });

  testWidgets('long-press hides reminders when signed out', (tester) async {
    await tester.pumpWidget(
      await buildTestable(
        overrides: [reminder.reminderServiceProvider.overrideWithValue(null)],
      ),
    );
    await tester.pumpAndSettle();

    await tester.longPress(find.byKey(const ValueKey('inbox-row-m1')));
    await tester.pumpAndSettle();
    expect(find.text('Follow thread'), findsOneWidget);
    expect(find.text('Remind me'), findsNothing);
  });

  testWidgets('swiping an inbox row reveals and runs its row actions', (
    tester,
  ) async {
    await tester.pumpWidget(
      await buildTestable(
        readContexts: {'ch1': now, 'ch2': now, 'thread:$threadIdRoot1': now},
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
        readContexts: {'ch1': now, 'ch2': now, 'thread:$threadIdRoot1': now},
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
      find.descendant(of: action, matching: find.byIcon(BuzzIcons.mail)),
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
        readContexts: {'ch1': now, 'ch2': now, 'thread:$threadIdRoot1': now},
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

  testWidgets('falls back to a compact npub when user is not cached', (
    tester,
  ) async {
    await tester.pumpWidget(await buildTestable(users: const {}));
    await tester.pumpAndSettle();

    // Sender label falls back to the compact npub of the author's key.
    expect(
      find.text(
        'a11ce00000000000000000000000000000000000000000000000000000000000',
      ),
      findsNothing,
    );
    expect(find.text('npub15yw\u2026ccpw'), findsOneWidget);
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

class _RecordingUserCacheNotifier extends _FakeUserCacheNotifier {
  _RecordingUserCacheNotifier(super._users);
  final preloaded = <String>{};

  @override
  Future<bool> preload(List<String> pubkeys) async {
    preloaded.addAll(pubkeys);
    return true;
  }
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

  int refreshes = 0;

  @override
  Future<List<Reminder>> build() async => _reminders;

  @override
  Future<void> refresh() async => refreshes++;
}

class _FakeComposeDraftsNotifier extends ComposeDraftsNotifier {
  final List<ComposeDraft> _drafts;
  _FakeComposeDraftsNotifier(this._drafts);

  @override
  List<ComposeDraft> build() => _drafts;
}

class _CapturingReminderService extends reminder.ReminderService {
  _CapturingReminderService._(nostr.Keys keys)
    : super(
        signedEventRelay: SignedEventRelay(
          session: RelaySessionNotifier(),
          nsec: keys.nsec,
        ),
        crypto: reminder.ReminderCrypto(keys.nsec, keys.public),
      );

  factory _CapturingReminderService() =>
      _CapturingReminderService._(nostr.Keys.generate());

  final saved =
      <({reminder.ReminderTarget target, String? note, String? dTag})>[];

  @override
  Future<void> createReminder({
    required reminder.ReminderTarget target,
    required int notBefore,
    String? note,
    String? replacingDTag,
  }) async {
    saved.add((target: target, note: note, dTag: replacingDTag));
  }
}
