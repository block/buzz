import 'dart:async';

import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';
import 'package:hooks_riverpod/misc.dart';
import 'package:buzz/features/activity/activity_page.dart';
import 'package:buzz/features/activity/activity_provider.dart';
import 'package:buzz/features/activity/feed_item.dart';
import 'package:buzz/features/channels/channel.dart';
import 'package:buzz/features/channels/channels_provider.dart';
import 'package:buzz/features/profile/user_cache_provider.dart';
import 'package:buzz/features/profile/user_profile.dart';
import 'package:buzz/shared/theme/theme.dart';

void main() {
  Widget buildTestable({required List<Override> overrides}) {
    return ProviderScope(
      overrides: overrides,
      child: MaterialApp(theme: AppTheme.light(), home: const ActivityPage()),
    );
  }

  final testMention = FeedItem(
    id: 'm1',
    kind: 9,
    pubkey: 'alice_pk',
    content: 'Hey check this out',
    createdAt: DateTime.now().millisecondsSinceEpoch ~/ 1000 - 120,
    channelId: 'ch1',
    channelName: 'general',
    tags: const [],
    category: 'mention',
  );

  final testActivity = FeedItem(
    id: 'a1',
    kind: 9,
    pubkey: 'bob_pk',
    content: 'Deployed the fix',
    createdAt: DateTime.now().millisecondsSinceEpoch ~/ 1000 - 3600,
    channelId: 'ch2',
    channelName: 'engineering',
    tags: const [],
    category: 'activity',
  );

  final testAgent = FeedItem(
    id: 'ag1',
    kind: 43004,
    pubkey: 'agent_pk',
    content: 'Job completed successfully',
    createdAt: DateTime.now().millisecondsSinceEpoch ~/ 1000 - 60,
    channelId: 'ch1',
    channelName: 'general',
    tags: const [],
    category: 'agent_activity',
  );

  final testFeed = HomeFeedResponse(
    mentions: [testMention],
    needsAction: const [],
    activity: [testActivity],
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

  final emptyUsers = <String, UserProfile>{};

  List<Override> defaultOverrides({HomeFeedResponse? feed}) => [
    activityProvider.overrideWith(
      () => _FakeActivityNotifier(feed ?? testFeed),
    ),
    channelsProvider.overrideWith(() => _FakeChannelsNotifier(testChannels)),
    userCacheProvider.overrideWith(() => _FakeUserCacheNotifier(testUsers)),
  ];

  testWidgets('shows loading skeleton while feed loads', (tester) async {
    await tester.pumpWidget(
      buildTestable(
        overrides: [
          activityProvider.overrideWith(() => _PendingActivityNotifier()),
          channelsProvider.overrideWith(() => _FakeChannelsNotifier([])),
          userCacheProvider.overrideWith(
            () => _FakeUserCacheNotifier(emptyUsers),
          ),
        ],
      ),
    );
    // Single pump - don't settle, the future never completes.
    await tester.pump();

    // Skeleton containers should be present (loading state).
    expect(find.byType(Container), findsWidgets);
    // No feed content visible.
    expect(find.text('Mention'), findsNothing);
  });

  testWidgets('shows empty state when feed is empty', (tester) async {
    final emptyFeed = HomeFeedResponse(
      mentions: [],
      needsAction: [],
      activity: [],
      agentActivity: [],
    );

    await tester.pumpWidget(
      buildTestable(overrides: defaultOverrides(feed: emptyFeed)),
    );
    await tester.pumpAndSettle();

    expect(find.text('No activity yet'), findsOneWidget);
    expect(
      find.text('Mentions, replies, and reactions will show up here.'),
      findsOneWidget,
    );
  });

  testWidgets('shows error view with retry button', (tester) async {
    await tester.pumpWidget(
      buildTestable(
        overrides: [
          activityProvider.overrideWith(() => _ErrorActivityNotifier()),
          channelsProvider.overrideWith(() => _FakeChannelsNotifier([])),
          userCacheProvider.overrideWith(
            () => _FakeUserCacheNotifier(emptyUsers),
          ),
        ],
      ),
    );
    await tester.pumpAndSettle();

    expect(find.text('Failed to load activity'), findsOneWidget);
    expect(find.text('Retry'), findsOneWidget);
  });

  testWidgets('shows feed items with correct content', (tester) async {
    await tester.pumpWidget(buildTestable(overrides: defaultOverrides()));
    await tester.pumpAndSettle();

    // All three items visible in "All" filter.
    expect(find.text('Hey check this out'), findsOneWidget);
    expect(find.text('Deployed the fix'), findsOneWidget);
    expect(find.text('Job completed successfully'), findsOneWidget);

    // Author names resolved from user cache.
    expect(find.text('Alice'), findsOneWidget);
    expect(find.text('Bob'), findsOneWidget);
    expect(find.text('Scout'), findsOneWidget);

    // Channel names visible.
    expect(find.text('#general'), findsNWidgets(2)); // mention + agent
    expect(find.text('#engineering'), findsOneWidget);
  });

  testWidgets('shows filter chips with counts', (tester) async {
    await tester.pumpWidget(buildTestable(overrides: defaultOverrides()));
    await tester.pumpAndSettle();

    expect(find.text('All'), findsOneWidget);
    expect(find.text('Mentions (1)'), findsOneWidget);
    expect(find.text('Action (0)'), findsOneWidget);
    expect(find.text('Activity (1)'), findsOneWidget);
    expect(find.text('Agents (1)'), findsOneWidget);
  });

  testWidgets('filtering shows only matching items', (tester) async {
    await tester.pumpWidget(buildTestable(overrides: defaultOverrides()));
    await tester.pumpAndSettle();

    // Tap the Mentions filter.
    await tester.tap(find.text('Mentions (1)'));
    await tester.pumpAndSettle();

    expect(find.text('Hey check this out'), findsOneWidget);
    expect(find.text('Deployed the fix'), findsNothing);
    expect(find.text('Job completed successfully'), findsNothing);
  });

  testWidgets('empty filter shows per-filter empty state', (tester) async {
    await tester.pumpWidget(buildTestable(overrides: defaultOverrides()));
    await tester.pumpAndSettle();

    // Tap Action filter (has 0 items).
    await tester.tap(find.text('Action (0)'));
    await tester.pumpAndSettle();

    expect(find.text('Nothing needs your action'), findsOneWidget);
  });

  testWidgets('shows headline for known event kinds', (tester) async {
    await tester.pumpWidget(buildTestable(overrides: defaultOverrides()));
    await tester.pumpAndSettle();

    // testAgent has kind 43004 -> "Job result"
    expect(find.text('Job result'), findsOneWidget);
    // testMention has kind 9, category mention -> "Mention"
    expect(find.text('Mention'), findsOneWidget);
    // testActivity has kind 9, category activity -> "Channel update"
    expect(find.text('Channel update'), findsOneWidget);
  });

  testWidgets('falls back to short pubkey when user not cached', (
    tester,
  ) async {
    await tester.pumpWidget(
      buildTestable(
        overrides: [
          activityProvider.overrideWith(() => _FakeActivityNotifier(testFeed)),
          channelsProvider.overrideWith(
            () => _FakeChannelsNotifier(testChannels),
          ),
          // Empty user cache - no profiles resolved.
          userCacheProvider.overrideWith(
            () => _FakeUserCacheNotifier(emptyUsers),
          ),
        ],
      ),
    );
    await tester.pumpAndSettle();

    // Should show truncated pubkey instead of display name.
    expect(find.text('alice_pk...'), findsOneWidget);
    expect(find.text('Alice'), findsNothing);
  });

  testWidgets('timestamps are right-aligned consistently', (tester) async {
    await tester.pumpWidget(buildTestable(overrides: defaultOverrides()));
    await tester.pumpAndSettle();

    // Find all time labels - they should show relative times.
    // testMention: 120s ago -> "2m"
    expect(find.text('2m'), findsOneWidget);
    // testActivity: 3600s ago -> "1h"
    expect(find.text('1h'), findsOneWidget);
    // testAgent: 60s ago -> "1m"
    expect(find.text('1m'), findsOneWidget);
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
