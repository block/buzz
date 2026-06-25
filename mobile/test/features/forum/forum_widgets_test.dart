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
import 'package:buzz/features/profile/user_cache_provider.dart';
import 'package:buzz/features/profile/user_profile.dart';
import 'package:buzz/shared/relay/relay.dart';
import 'package:buzz/shared/theme/theme.dart';

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
}) {
  return ProviderScope(
    overrides: [
      userCacheProvider.overrideWith(() => _FakeUserCacheNotifier(users)),
    ],
    child: MaterialApp(
      theme: AppTheme.light(),
      home: Scaffold(
        body: ForumPostCard(
          post: post,
          currentPubkey: currentPubkey,
          onTap: onTap ?? () {},
          onDelete: onDelete,
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

Widget _buildThreadPage({
  required ForumThreadResponse threadResponse,
  String postEventId = 'post1',
  String? currentPubkey = 'self',
  bool isMember = true,
  bool isArchived = false,
  Map<String, UserProfile> users = const {},
}) {
  return ProviderScope(
    overrides: [
      userCacheProvider.overrideWith(() => _FakeUserCacheNotifier(users)),
      profileProvider.overrideWith(() => _FakeProfileNotifier()),
      forumThreadProvider((
        channelId: _channelId,
        eventId: postEventId,
      )).overrideWith((ref) async => threadResponse),
      relayClientProvider.overrideWithValue(
        RelayClient(baseUrl: 'http://localhost:3000'),
      ),
    ],
    child: MaterialApp(
      theme: AppTheme.light(),
      home: ForumThreadPage(
        channelId: _channelId,
        postEventId: postEventId,
        currentPubkey: currentPubkey,
        isMember: isMember,
        isArchived: isArchived,
      ),
    ),
  );
}

void main() {
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
