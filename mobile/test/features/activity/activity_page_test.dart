import 'dart:async';

import 'package:buzz/features/activity/activity_page.dart';
import 'package:buzz/features/activity/activity_provider.dart';
import 'package:buzz/features/activity/feed_item.dart';
import 'package:buzz/features/activity/reminders_provider.dart';
import 'package:buzz/features/channels/channel.dart';
import 'package:buzz/features/channels/channel_detail_page.dart';
import 'package:buzz/features/channels/channels_provider.dart';
import 'package:buzz/features/channels/read_state/read_state_provider.dart';
import 'package:buzz/features/profile/user_cache_provider.dart';
import 'package:buzz/features/profile/user_profile.dart';
import 'package:buzz/shared/theme/theme.dart';
import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';
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
    'alice_pk': const UserProfile(pubkey: 'alice_pk', displayName: 'Alice'),
    'bob_pk': const UserProfile(pubkey: 'bob_pk', displayName: 'Bob'),
    'agent_pk': const UserProfile(pubkey: 'agent_pk', displayName: 'Scout'),
  };

  Future<Widget> buildTestable({
    HomeFeedResponse? feed,
    ActivityNotifier Function()? activityNotifier,
    Map<String, UserProfile>? users,
    Map<String, int> readContexts = const {},
    List<Channel>? channels,
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
      child: MaterialApp(theme: AppTheme.light(), home: const ActivityPage()),
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

  testWidgets('shows error view with retry button', (tester) async {
    await tester.pumpWidget(
      await buildTestable(activityNotifier: _ErrorActivityNotifier.new),
    );
    await tester.pumpAndSettle();

    expect(find.text('Failed to load activity'), findsOneWidget);
    expect(find.text('Retry'), findsOneWidget);
  });

  testWidgets('rows lead with sender, contextual label, and preview', (
    tester,
  ) async {
    await tester.pumpWidget(await buildTestable());
    await tester.pumpAndSettle();

    // Sender names resolved from the user cache.
    expect(find.text('Alice'), findsOneWidget);
    expect(find.text('Bob'), findsOneWidget);
    expect(find.text('Scout'), findsOneWidget);

    // Contextual labels + channel chips.
    expect(find.text('Mentioned in'), findsOneWidget);
    expect(find.text('Thread in'), findsOneWidget);
    expect(find.text('#general'), findsNWidgets(2)); // mention + agent
    expect(find.text('#engineering'), findsOneWidget);

    // Message previews.
    expect(find.textContaining('Hey check this out'), findsOneWidget);
    expect(find.textContaining('Deployed the fix'), findsOneWidget);

    // Sender uses the compact label scale (labelMedium), not a
    // headline-like title scale.
    final senderText = tester.widget<Text>(find.text('Alice'));
    final textTheme = Theme.of(tester.element(find.text('Alice'))).textTheme;
    expect(senderText.style?.fontSize, textTheme.labelMedium?.fontSize);
    expect(
      senderText.style!.fontSize!,
      lessThan(textTheme.titleSmall!.fontSize!),
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
    await tester.tap(find.text('Mark unread'));
    await tester.pumpAndSettle();

    expect(find.byKey(const ValueKey('inbox-unread-dot-m1')), findsOneWidget);
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
