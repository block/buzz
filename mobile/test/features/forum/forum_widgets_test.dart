import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';
import 'package:buzz/features/channels/channel.dart';
import 'package:buzz/features/forum/forum_models.dart';
import 'package:buzz/features/forum/forum_post_card.dart';
import 'package:buzz/features/forum/forum_posts_view.dart';
import 'package:buzz/features/forum/forum_provider.dart';
import 'package:buzz/features/forum/forum_thread_page.dart';
import 'package:buzz/features/profile/profile_provider.dart';
import 'package:buzz/shared/mentions/agent_identity_provider.dart';
import 'package:buzz/shared/profile/user_cache_provider.dart';
import 'package:buzz/shared/profile/user_profile.dart';
import 'package:buzz/shared/relay/relay.dart';
import 'package:buzz/shared/theme/theme.dart';
import 'package:buzz/shared/widgets/avatar_image.dart';
import 'package:shared_preferences/shared_preferences.dart';

const _channelId = 'forum-channel';

final _forumChannel = Channel(
  id: _channelId,
  name: 'design-forum',
  channelType: 'forum',
  visibility: 'open',
  description: '',
  createdBy: 'abc123',
  createdAt: DateTime(2025),
  memberCount: 5,
  isMember: true,
);

ForumPost _makePost({
  String eventId = 'post1',
  String pubkey = 'alice',
  String content = 'Hello forum',
  int createdAt = 1000,
  List<List<String>> tags = const [
    ['h', 'forum-channel'],
  ],
  ForumThreadSummary? threadSummary,
}) => ForumPost(
  eventId: eventId,
  pubkey: pubkey,
  content: content,
  kind: 45001,
  createdAt: createdAt,
  channelId: _channelId,
  tags: tags,
  threadSummary: threadSummary,
);

const _aliceProfile = UserProfile(pubkey: 'alice', displayName: 'Alice');

void _setSurfaceSize(WidgetTester tester, Size size) {
  tester.view.devicePixelRatio = 1.0;
  tester.view.physicalSize = size;
}

Widget _buildPostCard({
  required ForumPost post,
  String? currentPubkey = 'self',
  Map<String, UserProfile> users = const {},
  VoidCallback? onTap,
  void Function(String)? onDelete,
  TextScaler textScaler = TextScaler.noScaling,
  Set<String> knownAgentPubkeys = const {},
}) {
  return ProviderScope(
    overrides: [
      userCacheProvider.overrideWith(() => _FakeUserCacheNotifier(users)),
      knownAgentPubkeysProvider.overrideWithValue(knownAgentPubkeys),
    ],
    child: MaterialApp(
      theme: AppTheme.light(),
      home: Builder(
        builder: (context) => MediaQuery(
          data: MediaQuery.of(context).copyWith(textScaler: textScaler),
          child: Scaffold(
            body: ForumPostCard(
              post: post,
              currentPubkey: currentPubkey,
              onTap: onTap ?? () {},
              onDelete: onDelete,
            ),
          ),
        ),
      ),
    ),
  );
}

Widget _buildPostsView({
  required ForumPostsResponse postsResponse,
  Channel? channel,
  Map<String, UserProfile> users = const {},
}) {
  final ch = channel ?? _forumChannel;
  return ProviderScope(
    overrides: [
      userCacheProvider.overrideWith(() => _FakeUserCacheNotifier(users)),
      profileProvider.overrideWith(() => _FakeProfileNotifier()),
      forumPostsProvider(ch.id).overrideWith((ref) async => postsResponse),
      relayClientProvider.overrideWithValue(
        RelayClient(baseUrl: 'http://localhost:3000'),
      ),
    ],
    child: MaterialApp(
      theme: AppTheme.light(),
      home: Scaffold(
        body: ForumPostsView(channel: ch, currentPubkey: 'self'),
      ),
    ),
  );
}

/// Shared mock prefs for the compose bar's draft store. Initialized in
/// [main].
late SharedPreferences _testPrefs;

Widget _buildThreadPage({
  required ForumThreadResponse threadResponse,
  String postEventId = 'post1',
  String? currentPubkey = 'self',
  bool isMember = true,
  bool isArchived = false,
  Map<String, UserProfile> users = const {},
  Set<String> knownAgentPubkeys = const {},
  Set<String> channelBotPubkeys = const {},
  TextScaler textScaler = TextScaler.noScaling,
}) {
  return ProviderScope(
    overrides: [
      userCacheProvider.overrideWith(() => _FakeUserCacheNotifier(users)),
      knownAgentPubkeysProvider.overrideWithValue(knownAgentPubkeys),
      channelBotPubkeysProvider(
        _channelId,
      ).overrideWith((ref) async => channelBotPubkeys),
      profileProvider.overrideWith(() => _FakeProfileNotifier()),
      forumThreadProvider((
        channelId: _channelId,
        eventId: postEventId,
      )).overrideWith((ref) async => threadResponse),
      savedPrefsProvider.overrideWithValue(_testPrefs),
      relayClientProvider.overrideWithValue(
        RelayClient(baseUrl: 'http://localhost:3000'),
      ),
    ],
    child: MaterialApp(
      theme: AppTheme.light(),
      home: Builder(
        builder: (context) => MediaQuery(
          data: MediaQuery.of(context).copyWith(textScaler: textScaler),
          child: ForumThreadPage(
            channelId: _channelId,
            postEventId: postEventId,
            currentPubkey: currentPubkey,
            isMember: isMember,
            isArchived: isArchived,
          ),
        ),
      ),
    ),
  );
}

void main() {
  setUp(() async {
    SharedPreferences.setMockInitialValues({});
    _testPrefs = await SharedPreferences.getInstance();
  });

  test('cancels a captured forum delivery after the community changes', () {
    final container = ProviderContainer();
    addTearDown(container.dispose);
    container
        .read(relayConfigProvider.notifier)
        .update(baseUrl: 'https://first.example');
    final delivery = ForumEventDelivery.capture(container);

    container
        .read(relayConfigProvider.notifier)
        .update(baseUrl: 'https://second.example');

    expect(
      delivery.createPost(channelId: _channelId, content: 'Queued post'),
      throwsA(
        isA<StateError>().having(
          (error) => error.message,
          'message',
          contains('active community changed'),
        ),
      ),
    );
  });

  group('ForumPostCard', () {
    testWidgets('renders author name and content', (tester) async {
      await tester.pumpWidget(
        _buildPostCard(
          post: _makePost(),
          users: const {'alice': _aliceProfile},
        ),
      );
      await tester.pumpAndSettle();

      expect(find.text('Alice'), findsOneWidget);
      expect(find.text('Hello forum'), findsOneWidget);
    });

    testWidgets('shows truncated pubkey when no profile', (tester) async {
      await tester.pumpWidget(
        _buildPostCard(post: _makePost(pubkey: 'abcdef1234567890')),
      );
      await tester.pumpAndSettle();

      expect(find.text('abcdef12\u2026'), findsOneWidget);
    });

    testWidgets('uses directory classification for uncached author avatar', (
      tester,
    ) async {
      await tester.pumpWidget(
        _buildPostCard(
          post: _makePost(pubkey: 'directory-agent'),
          knownAgentPubkeys: const {'directory-agent'},
        ),
      );
      await tester.pumpAndSettle();

      expect(
        tester.widget<AvatarImage>(find.byType(AvatarImage)).isAgent,
        isTrue,
      );
    });

    testWidgets('keeps human author avatar circular', (tester) async {
      await tester.pumpWidget(
        _buildPostCard(
          post: _makePost(),
          users: const {'alice': _aliceProfile},
        ),
      );
      await tester.pumpAndSettle();

      expect(
        tester.widget<AvatarImage>(find.byType(AvatarImage)).isAgent,
        isFalse,
      );
    });

    testWidgets(
      'constrains an older timestamp at large accessible text sizes',
      (tester) async {
        _setSurfaceSize(tester, const Size(240, 600));
        addTearDown(() {
          tester.view.resetPhysicalSize();
          tester.view.resetDevicePixelRatio();
        });

        await tester.pumpWidget(
          _buildPostCard(
            post: _makePost(
              createdAt:
                  DateTime.utc(2025, 12, 31, 12).millisecondsSinceEpoch ~/ 1000,
            ),
            users: const {
              'alice': UserProfile(
                pubkey: 'alice',
                displayName: 'A very long display name',
              ),
            },
            textScaler: const TextScaler.linear(2),
          ),
        );
        await tester.pumpAndSettle();

        final timestamp = tester.widget<Text>(find.text('12/31/2025'));
        expect(timestamp.maxLines, 1);
        expect(timestamp.overflow, TextOverflow.ellipsis);
        expect(tester.takeException(), isNull);
      },
    );

    testWidgets('gives the author unused timestamp width', (tester) async {
      _setSurfaceSize(tester, const Size(320, 600));
      addTearDown(() {
        tester.view.resetPhysicalSize();
        tester.view.resetDevicePixelRatio();
      });
      final createdAt = DateTime.now().millisecondsSinceEpoch ~/ 1000 - 120;
      const displayName = 'A moderately long forum author name';

      await tester.pumpWidget(
        _buildPostCard(
          post: _makePost(createdAt: createdAt),
          users: const {
            'alice': UserProfile(pubkey: 'alice', displayName: displayName),
          },
        ),
      );
      await tester.pumpAndSettle();

      expect(tester.getSize(find.text(displayName)).width, greaterThan(150));
      expect(find.text('2m ago'), findsOneWidget);
      expect(tester.takeException(), isNull);
    });

    testWidgets('truncates long content', (tester) async {
      final longContent = 'A' * 300;
      await tester.pumpWidget(
        _buildPostCard(post: _makePost(content: longContent)),
      );
      await tester.pumpAndSettle();

      // Should show 200 chars + "..."
      expect(find.textContaining('${'A' * 200}...'), findsOneWidget);
    });

    testWidgets('shows reply count with correct pluralization', (tester) async {
      await tester.pumpWidget(
        _buildPostCard(
          post: _makePost(
            threadSummary: const ForumThreadSummary(
              replyCount: 1,
              descendantCount: 1,
              participants: [],
            ),
          ),
        ),
      );
      await tester.pumpAndSettle();
      expect(find.text('1 reply'), findsOneWidget);

      await tester.pumpWidget(
        _buildPostCard(
          post: _makePost(
            threadSummary: const ForumThreadSummary(
              replyCount: 5,
              descendantCount: 5,
              participants: [],
            ),
          ),
        ),
      );
      await tester.pumpAndSettle();
      expect(find.text('5 replies'), findsOneWidget);
    });

    testWidgets('hides thread summary when reply count is 0', (tester) async {
      await tester.pumpWidget(
        _buildPostCard(
          post: _makePost(
            threadSummary: const ForumThreadSummary(
              replyCount: 0,
              descendantCount: 0,
              participants: [],
            ),
          ),
        ),
      );
      await tester.pumpAndSettle();
      expect(find.text('0 replies'), findsNothing);
    });

    testWidgets('calls onTap when tapped', (tester) async {
      var tapped = false;
      await tester.pumpWidget(
        _buildPostCard(post: _makePost(), onTap: () => tapped = true),
      );
      await tester.pumpAndSettle();

      await tester.tap(find.byType(ForumPostCard));
      expect(tapped, isTrue);
    });

    testWidgets('keeps media previews non-interactive in the post list', (
      tester,
    ) async {
      var tapped = false;
      const imageUrl = 'https://example.com/media/card.png';

      await tester.pumpWidget(
        _buildPostCard(
          post: _makePost(
            content: '![image]($imageUrl)',
            tags: const [
              ['h', _channelId],
              [
                'imeta',
                'url https://example.com/media/card.png',
                'm image/png',
              ],
            ],
          ),
          onTap: () => tapped = true,
        ),
      );
      await tester.pumpAndSettle();

      final preview = find.byKey(
        const ValueKey(
          'message-media-image-preview:https://example.com/media/card.png',
        ),
      );

      await tester.tapAt(tester.getCenter(preview));
      await tester.pumpAndSettle();

      expect(tapped, isTrue);
      expect(
        find.byKey(const ValueKey('message-media-image-viewer')),
        findsNothing,
      );
    });

    testWidgets('long press opens action sheet with Copy text', (tester) async {
      await tester.pumpWidget(_buildPostCard(post: _makePost()));
      await tester.pumpAndSettle();

      await tester.longPress(find.byType(ForumPostCard));
      await tester.pumpAndSettle();

      expect(find.text('Copy text'), findsOneWidget);
    });

    testWidgets('long press shows Delete only for own posts', (tester) async {
      // Own post — Delete should appear.
      await tester.pumpWidget(
        _buildPostCard(
          post: _makePost(pubkey: 'self'),
          currentPubkey: 'self',
          onDelete: (_) {},
        ),
      );
      await tester.pumpAndSettle();

      await tester.longPress(find.byType(ForumPostCard));
      await tester.pumpAndSettle();
      expect(find.text('Delete post'), findsOneWidget);

      // Dismiss sheet.
      await tester.tapAt(Offset.zero);
      await tester.pumpAndSettle();

      // Other's post — Delete should NOT appear.
      await tester.pumpWidget(
        _buildPostCard(
          post: _makePost(pubkey: 'other'),
          currentPubkey: 'self',
          onDelete: (_) {},
        ),
      );
      await tester.pumpAndSettle();

      await tester.longPress(find.byType(ForumPostCard));
      await tester.pumpAndSettle();
      expect(find.text('Delete post'), findsNothing);
    });

    testWidgets('delete confirmation dialog triggers onDelete', (tester) async {
      String? deletedId;
      await tester.pumpWidget(
        _buildPostCard(
          post: _makePost(pubkey: 'self', eventId: 'evt-to-delete'),
          currentPubkey: 'self',
          onDelete: (id) => deletedId = id,
        ),
      );
      await tester.pumpAndSettle();

      // Long press → action sheet.
      await tester.longPress(find.byType(ForumPostCard));
      await tester.pumpAndSettle();

      // Tap Delete post.
      await tester.tap(find.text('Delete post'));
      await tester.pumpAndSettle();

      // Confirmation dialog appears.
      expect(find.text('This cannot be undone.'), findsOneWidget);

      // Tap Delete button.
      await tester.tap(find.widgetWithText(FilledButton, 'Delete'));
      await tester.pumpAndSettle();

      expect(deletedId, 'evt-to-delete');
    });
  });

  group('ForumPostsView', () {
    testWidgets('shows empty state for members', (tester) async {
      await tester.pumpWidget(
        _buildPostsView(postsResponse: const ForumPostsResponse(posts: [])),
      );
      await tester.pumpAndSettle();

      expect(find.text('No posts yet'), findsOneWidget);
      expect(
        find.text('Start a discussion by creating the first post.'),
        findsOneWidget,
      );
    });

    testWidgets('shows empty state for non-members', (tester) async {
      await tester.pumpWidget(
        _buildPostsView(
          postsResponse: const ForumPostsResponse(posts: []),
          channel: Channel(
            id: _channelId,
            name: 'design-forum',
            channelType: 'forum',
            visibility: 'open',
            description: '',
            createdBy: 'abc123',
            createdAt: DateTime(2025),
            memberCount: 5,
            isMember: false,
          ),
        ),
      );
      await tester.pumpAndSettle();

      expect(find.text('Join this forum to create posts.'), findsOneWidget);
    });

    testWidgets('shows FAB for members', (tester) async {
      await tester.pumpWidget(
        _buildPostsView(postsResponse: const ForumPostsResponse(posts: [])),
      );
      await tester.pumpAndSettle();

      expect(find.byType(FloatingActionButton), findsOneWidget);
      expect(find.byTooltip('New post'), findsOneWidget);
    });

    testWidgets('hides FAB for non-members', (tester) async {
      await tester.pumpWidget(
        _buildPostsView(
          postsResponse: const ForumPostsResponse(posts: []),
          channel: Channel(
            id: _channelId,
            name: 'design-forum',
            channelType: 'forum',
            visibility: 'open',
            description: '',
            createdBy: 'abc123',
            createdAt: DateTime(2025),
            memberCount: 5,
            isMember: false,
          ),
        ),
      );
      await tester.pumpAndSettle();

      expect(find.byType(FloatingActionButton), findsNothing);
    });

    testWidgets('renders post list', (tester) async {
      await tester.pumpWidget(
        _buildPostsView(
          postsResponse: ForumPostsResponse(
            posts: [
              _makePost(content: 'First post'),
              _makePost(eventId: 'post2', content: 'Second post'),
            ],
          ),
          users: const {'alice': _aliceProfile},
        ),
      );
      await tester.pumpAndSettle();

      expect(find.text('First post'), findsOneWidget);
      expect(find.text('Second post'), findsOneWidget);
    });
  });

  group('ForumThreadPage', () {
    AvatarImage avatarIn(WidgetTester tester, Key key) =>
        tester.widget<AvatarImage>(
          find.descendant(
            of: find.byKey(key),
            matching: find.byType(AvatarImage),
          ),
        );

    testWidgets(
      'uses directory classification for an uncached original author',
      (tester) async {
        await tester.pumpWidget(
          _buildThreadPage(
            threadResponse: ForumThreadResponse(
              post: _makePost(pubkey: 'directory-agent'),
              replies: const [],
              totalReplies: 0,
            ),
            knownAgentPubkeys: const {'directory-agent'},
          ),
        );
        await tester.pumpAndSettle();

        expect(
          avatarIn(
            tester,
            const ValueKey('forum-original-avatar-post1'),
          ).isAgent,
          isTrue,
        );
      },
    );

    testWidgets('uses bot-role classification for an uncached reply author', (
      tester,
    ) async {
      await tester.pumpWidget(
        _buildThreadPage(
          threadResponse: ForumThreadResponse(
            post: _makePost(),
            replies: const [
              ThreadReply(
                eventId: 'bot-reply',
                pubkey: 'channel-bot',
                content: 'Automated reply',
                kind: 45003,
                createdAt: 2000,
                channelId: _channelId,
                tags: [
                  ['h', _channelId],
                ],
                depth: 1,
              ),
            ],
            totalReplies: 1,
          ),
          users: const {'alice': _aliceProfile},
          channelBotPubkeys: const {'channel-bot'},
        ),
      );
      await tester.pumpAndSettle();

      expect(
        avatarIn(
          tester,
          const ValueKey('forum-reply-avatar-bot-reply'),
        ).isAgent,
        isTrue,
      );
      expect(
        avatarIn(tester, const ValueKey('forum-original-avatar-post1')).isAgent,
        isFalse,
      );
    });

    testWidgets('shows original post and replies header', (tester) async {
      await tester.pumpWidget(
        _buildThreadPage(
          threadResponse: ForumThreadResponse(
            post: _makePost(content: 'Thread root'),
            replies: const [],
            totalReplies: 0,
          ),
          users: const {'alice': _aliceProfile},
        ),
      );
      await tester.pumpAndSettle();

      expect(find.text('Thread'), findsOneWidget); // App bar title
      expect(find.text('0 replies'), findsOneWidget);
      expect(
        find.text('No replies yet. Be the first to respond.'),
        findsOneWidget,
      );
    });

    testWidgets('shows reply count with replies', (tester) async {
      await tester.pumpWidget(
        _buildThreadPage(
          threadResponse: ForumThreadResponse(
            post: _makePost(),
            replies: [
              const ThreadReply(
                eventId: 'r1',
                pubkey: 'bob',
                content: 'Great post!',
                kind: 45003,
                createdAt: 2000,
                channelId: _channelId,
                tags: [
                  ['h', _channelId],
                ],
                depth: 1,
              ),
            ],
            totalReplies: 1,
          ),
          users: const {
            'alice': _aliceProfile,
            'bob': UserProfile(pubkey: 'bob', displayName: 'Bob'),
          },
        ),
      );
      await tester.pumpAndSettle();

      expect(find.text('1 reply'), findsOneWidget);
      expect(find.text('Bob'), findsOneWidget);
    });

    testWidgets('constrains post and reply timestamps at large text sizes', (
      tester,
    ) async {
      final oldTimestamp =
          DateTime.utc(2025, 12, 31, 12).millisecondsSinceEpoch ~/ 1000;

      await tester.pumpWidget(
        _buildThreadPage(
          threadResponse: ForumThreadResponse(
            post: _makePost(createdAt: oldTimestamp),
            replies: [
              ThreadReply(
                eventId: 'old-reply',
                pubkey: 'bob',
                content: 'An older reply',
                kind: 45003,
                createdAt: oldTimestamp,
                channelId: _channelId,
                tags: const [
                  ['h', _channelId],
                ],
                depth: 1,
              ),
            ],
            totalReplies: 1,
          ),
          users: const {
            'alice': _aliceProfile,
            'bob': UserProfile(
              pubkey: 'bob',
              displayName: 'A very long reply author name',
            ),
          },
          textScaler: const TextScaler.linear(2),
        ),
      );
      await tester.pumpAndSettle();

      final timestamps = tester.widgetList<Text>(find.text('12/31/2025'));
      expect(timestamps, hasLength(2));
      for (final timestamp in timestamps) {
        expect(timestamp.maxLines, 1);
        expect(timestamp.overflow, TextOverflow.ellipsis);
      }
      expect(tester.takeException(), isNull);
    });

    testWidgets('gives thread authors unused timestamp width', (tester) async {
      _setSurfaceSize(tester, const Size(320, 800));
      addTearDown(() {
        tester.view.resetPhysicalSize();
        tester.view.resetDevicePixelRatio();
      });
      final createdAt = DateTime.now().millisecondsSinceEpoch ~/ 1000 - 120;
      const postAuthor = 'A moderately long original author';
      const replyAuthor = 'A moderately long reply author';

      await tester.pumpWidget(
        _buildThreadPage(
          threadResponse: ForumThreadResponse(
            post: _makePost(createdAt: createdAt),
            replies: [
              ThreadReply(
                eventId: 'reply',
                pubkey: 'bob',
                content: 'A reply',
                kind: 45003,
                createdAt: createdAt,
                channelId: _channelId,
                tags: const [
                  ['h', _channelId],
                ],
                depth: 1,
              ),
            ],
            totalReplies: 1,
          ),
          users: const {
            'alice': UserProfile(pubkey: 'alice', displayName: postAuthor),
            'bob': UserProfile(pubkey: 'bob', displayName: replyAuthor),
          },
        ),
      );
      await tester.pumpAndSettle();

      expect(tester.getSize(find.text(postAuthor)).width, greaterThan(150));
      expect(tester.getSize(find.text(replyAuthor)).width, greaterThan(140));
      expect(find.text('2m ago'), findsNWidgets(2));
      expect(tester.takeException(), isNull);
    });

    testWidgets('shows compose bar for members', (tester) async {
      await tester.pumpWidget(
        _buildThreadPage(
          threadResponse: ForumThreadResponse(
            post: _makePost(),
            replies: const [],
            totalReplies: 0,
          ),
          isMember: true,
        ),
      );
      await tester.pumpAndSettle();

      expect(find.text('Reply to this post\u2026'), findsOneWidget);
    });

    testWidgets('renders media previews for forum posts', (tester) async {
      const imageUrl = 'https://example.com/media/forum.png';

      await tester.pumpWidget(
        _buildThreadPage(
          threadResponse: ForumThreadResponse(
            post: _makePost(
              content: '![image]($imageUrl)',
              tags: const [
                ['h', _channelId],
                [
                  'imeta',
                  'url https://example.com/media/forum.png',
                  'm image/png',
                ],
              ],
            ),
            replies: const [],
            totalReplies: 0,
          ),
          users: const {'alice': _aliceProfile},
        ),
      );
      await tester.pumpAndSettle();

      expect(
        find.byKey(
          const ValueKey(
            'message-media-image-preview:https://example.com/media/forum.png',
          ),
        ),
        findsOneWidget,
      );
    });

    testWidgets('keeps tall forum image previews bounded inline', (
      tester,
    ) async {
      _setSurfaceSize(tester, const Size(400, 800));
      addTearDown(() {
        tester.view.resetPhysicalSize();
        tester.view.resetDevicePixelRatio();
      });

      const imageUrl = 'https://example.com/media/forum-tall.png';

      await tester.pumpWidget(
        _buildThreadPage(
          threadResponse: ForumThreadResponse(
            post: _makePost(
              content: '![image]($imageUrl)',
              tags: const [
                ['h', _channelId],
                [
                  'imeta',
                  'url https://example.com/media/forum-tall.png',
                  'm image/png',
                  'dim 1200x2400',
                ],
              ],
            ),
            replies: const [],
            totalReplies: 0,
          ),
          users: const {'alice': _aliceProfile},
        ),
      );
      await tester.pumpAndSettle();

      final preview = find.byKey(
        const ValueKey(
          'message-media-image-preview:https://example.com/media/forum-tall.png',
        ),
      );
      final size = tester.getSize(preview);

      expect(size.height, closeTo(240, 0.1));
      expect(size.width, closeTo(120, 0.1));
    });

    testWidgets('hides compose bar for non-members', (tester) async {
      await tester.pumpWidget(
        _buildThreadPage(
          threadResponse: ForumThreadResponse(
            post: _makePost(),
            replies: const [],
            totalReplies: 0,
          ),
          isMember: false,
        ),
      );
      await tester.pumpAndSettle();

      expect(find.text('Reply to this post\u2026'), findsNothing);
    });

    testWidgets('shows 3-dot in app bar for own post', (tester) async {
      await tester.pumpWidget(
        _buildThreadPage(
          threadResponse: ForumThreadResponse(
            post: _makePost(pubkey: 'self'),
            replies: const [],
            totalReplies: 0,
          ),
          currentPubkey: 'self',
        ),
      );
      await tester.pumpAndSettle();

      expect(find.byTooltip('Post actions'), findsOneWidget);
    });

    testWidgets('hides 3-dot in app bar for others post', (tester) async {
      await tester.pumpWidget(
        _buildThreadPage(
          threadResponse: ForumThreadResponse(
            post: _makePost(pubkey: 'alice'),
            replies: const [],
            totalReplies: 0,
          ),
          currentPubkey: 'self',
        ),
      );
      await tester.pumpAndSettle();

      expect(find.byTooltip('Post actions'), findsNothing);
    });
  });
}

class _FakeUserCacheNotifier extends UserCacheNotifier {
  final Map<String, UserProfile> _users;
  _FakeUserCacheNotifier(this._users);

  @override
  Map<String, UserProfile> build() => _users;

  @override
  UserProfile? get(String pubkey) => _users[pubkey.toLowerCase()];
}

class _FakeProfileNotifier extends ProfileNotifier {
  @override
  Future<UserProfile?> build() async =>
      const UserProfile(pubkey: 'self', displayName: 'Self');
}
