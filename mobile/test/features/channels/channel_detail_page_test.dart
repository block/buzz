import 'dart:convert';

import 'package:flutter/foundation.dart';
import 'package:flutter/material.dart';
import 'package:flutter/rendering.dart' show ScrollDirection;
import 'package:flutter_test/flutter_test.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';
import 'package:lucide_icons_flutter/lucide_icons.dart';
import 'package:scrollable_positioned_list/scrollable_positioned_list.dart';
import 'package:buzz/features/channels/channel.dart';
import 'package:buzz/features/channels/channel_detail_page.dart';
import 'package:buzz/features/channels/channel_management_provider.dart';
import 'package:buzz/features/channels/channel_messages_provider.dart';
import 'package:buzz/features/channels/channel_typing_provider.dart';
import 'package:buzz/features/channels/date_formatters.dart';
import 'package:buzz/features/channels/day_divider.dart';
import 'package:buzz/features/channels/reaction_row.dart';
import 'package:buzz/features/channels/thread_detail_page.dart';
import 'package:buzz/features/channels/thread_replies_provider.dart';
import 'package:buzz/features/channels/timeline_message.dart';
import 'package:buzz/features/channels/channels_provider.dart';
import 'package:buzz/features/channels/read_state/read_state_provider.dart';
import 'package:buzz/features/channels/small_avatar.dart';
import 'package:buzz/features/profile/profile_provider.dart';
import 'package:buzz/features/profile/user_cache_provider.dart';
import 'package:buzz/features/profile/user_profile.dart';
import 'package:buzz/shared/relay/relay.dart';
import 'package:buzz/shared/theme/theme.dart';
import 'package:buzz/shared/widgets/skeleton.dart';
import 'package:shared_preferences/shared_preferences.dart';

const _channelId = 'test-channel';

/// Shared mock prefs for providers that read [savedPrefsProvider]
/// (e.g. the compose bar's draft store). Initialized in [main].
late SharedPreferences _testPrefs;

final _testChannel = Channel(
  id: _channelId,
  name: 'general',
  channelType: 'stream',
  visibility: 'open',
  description: 'General discussion',
  createdBy: 'abc123',
  createdAt: DateTime(2025),
  memberCount: 5,
  isMember: true,
);

NostrEvent _textMsg({
  required String id,
  required String pubkey,
  required String content,
  int createdAt = 1000,
  List<List<String>> extraTags = const [],
}) => NostrEvent(
  id: id,
  pubkey: pubkey,
  createdAt: createdAt,
  kind: EventKind.streamMessage,
  tags: [
    ['h', _channelId],
    ...extraTags,
  ],
  content: content,
  sig: '',
);

NostrEvent _systemMsg({
  required String id,
  required Map<String, dynamic> payload,
  int createdAt = 1000,
}) => NostrEvent(
  id: id,
  pubkey: 'relay',
  createdAt: createdAt,
  kind: EventKind.systemMessage,
  tags: [
    ['h', _channelId],
  ],
  content: jsonEncode(payload),
  sig: '',
);

NostrEvent _reaction({
  required String id,
  required String targetId,
  String pubkey = 'bob',
  int createdAt = 2000,
  String content = '👍',
}) => NostrEvent(
  id: id,
  pubkey: pubkey,
  createdAt: createdAt,
  kind: EventKind.reaction,
  tags: [
    ['h', _channelId],
    ['e', targetId],
  ],
  content: content,
  sig: '',
);

NostrEvent _deletion({
  required String id,
  required List<String> targetIds,
  int createdAt = 2000,
}) => NostrEvent(
  id: id,
  pubkey: 'abc123',
  createdAt: createdAt,
  kind: EventKind.deletion,
  tags: [
    ['h', _channelId],
    for (final t in targetIds) ['e', t],
  ],
  content: '',
  sig: '',
);

NostrEvent _edit({
  required String id,
  required String targetId,
  required String content,
  int createdAt = 2000,
}) => NostrEvent(
  id: id,
  pubkey: 'abc123',
  createdAt: createdAt,
  kind: EventKind.streamMessageEdit,
  tags: [
    ['h', _channelId],
    ['e', targetId],
  ],
  content: content,
  sig: '',
);

Widget _buildTestable({
  required List<NostrEvent> messages,
  List<TypingEntry> typing = const [],
  Map<String, UserProfile> users = const {},
  List<ChannelMember> members = const [],
  Channel? channel,
  List<Channel>? channels,
  _FakeChannelsNotifier? channelsNotifier,
  List<NavigatorObserver> navigatorObservers = const [],
  Future<List<ChannelMember>> Function()? loadMembers,
  ChannelActions Function(Ref ref)? createChannelActions,
  ReadStateNotifier? readStateNotifier,
  _FakeMessagesNotifier? messagesNotifier,
  String? canvasContent,
  String? initialMessageId,
  String? initialThreadRootId,
  Map<String, List<NostrEvent>> threadReplies = const {},
  TextScaler textScaler = TextScaler.noScaling,
  RelaySessionNotifier? relaySessionNotifier,
}) {
  final resolvedChannel = channel ?? _testChannel;
  final fakeChannelsNotifier =
      channelsNotifier ?? _FakeChannelsNotifier(channels ?? [resolvedChannel]);
  final fakeMessagesNotifier =
      messagesNotifier ?? _FakeMessagesNotifier(messages);
  return ProviderScope(
    overrides: [
      channelMessagesProvider(
        _channelId,
      ).overrideWith(() => fakeMessagesNotifier),
      channelTypingProvider(
        _channelId,
      ).overrideWith(() => _FakeTypingNotifier(typing)),
      userCacheProvider.overrideWith(() => _FakeUserCacheNotifier(users)),
      profileProvider.overrideWith(() => _FakeProfileNotifier()),
      channelsProvider.overrideWith(() => fakeChannelsNotifier),
      channelDetailsProvider(_channelId).overrideWith(
        (ref) async => ChannelDetails.fromChannel(resolvedChannel),
      ),
      channelCanvasProvider(_channelId).overrideWith(
        (ref) async => ChannelCanvas(
          content: canvasContent,
          updatedAt: null,
          authorPubkey: null,
        ),
      ),
      channelMembersProvider(_channelId).overrideWith(
        (ref) async => loadMembers != null ? loadMembers() : members,
      ),
      if (createChannelActions != null)
        channelActionsProvider.overrideWith(createChannelActions),
      if (readStateNotifier != null)
        readStateProvider.overrideWith(() => readStateNotifier),
      for (final entry in threadReplies.entries)
        threadRepliesProvider(
          ThreadRepliesArgs(channelId: _channelId, rootId: entry.key),
        ).overrideWith((ref) async => entry.value),
      // Stub the relay client provider so preloadMembers doesn't crash.
      relayClientProvider.overrideWithValue(
        RelayClient(baseUrl: 'http://localhost:3000'),
      ),
      if (relaySessionNotifier != null)
        relaySessionProvider.overrideWith(() => relaySessionNotifier),
      // Compose bar drafts persist through SharedPreferences.
      savedPrefsProvider.overrideWithValue(_testPrefs),
    ],
    child: MaterialApp(
      theme: AppTheme.light(),
      builder: (context, child) => MediaQuery(
        data: MediaQuery.of(context).copyWith(textScaler: textScaler),
        child: child!,
      ),
      navigatorObservers: navigatorObservers,
      home: ChannelDetailPage(
        channel: resolvedChannel,
        initialMessageId: initialMessageId,
        initialThreadRootId: initialThreadRootId,
      ),
    ),
  );
}

/// Finder that searches for text within RichText spans. [find.text] only
/// matches the top-level text property; this also searches nested TextSpans.
Finder findRichText(String text) {
  return find.byWidgetPredicate((widget) {
    if (widget is RichText) {
      return widget.text.toPlainText().contains(text);
    }
    return false;
  }, description: 'RichText containing "$text"');
}

double? effectiveFontSizeForText(
  InlineSpan span,
  String text, [
  TextStyle? inheritedStyle,
]) {
  if (span is! TextSpan) return null;
  final effectiveStyle = inheritedStyle?.merge(span.style) ?? span.style;
  if ((span.text ?? '').contains(text)) return effectiveStyle?.fontSize;
  for (final child in span.children ?? const <InlineSpan>[]) {
    final size = effectiveFontSizeForText(child, text, effectiveStyle);
    if (size != null) return size;
  }
  return null;
}

void main() {
  setUp(() async {
    SharedPreferences.setMockInitialValues({});
    _testPrefs = await SharedPreferences.getInstance();
  });

  group('ChannelDetailPage', () {
    testWidgets('debounces same-slot reconnect skeletons before revealing', (
      tester,
    ) async {
      final relaySession = _ReconnectingRelaySession();
      await tester.pumpWidget(
        _buildTestable(
          messages: [
            _textMsg(id: 'msg1', pubkey: 'alice', content: 'Existing message'),
          ],
          relaySessionNotifier: relaySession,
          readStateNotifier: _SynchronousReadStateNotifier(
            const ReadStateState(
              isReady: false,
              pubkey: 'self',
              contexts: {},
              version: 0,
            ),
          ),
        ),
      );
      await tester.pump();

      expect(find.text('Existing message'), findsOneWidget);
      expect(
        tester.widget<SkeletonReveal>(find.byType(SkeletonReveal)).loading,
        isFalse,
      );

      await tester.pump(const Duration(milliseconds: 1999));
      expect(
        tester.widget<SkeletonReveal>(find.byType(SkeletonReveal)).loading,
        isFalse,
      );

      await tester.pump(const Duration(milliseconds: 1));
      await tester.pump();
      final skeleton = find.byKey(
        const Key('channel-detail-connection-skeleton'),
      );
      expect(skeleton, findsOneWidget);
      expect(
        find.descendant(of: skeleton, matching: find.byType(SkeletonBar)),
        findsWidgets,
      );
      expect(find.text('Existing message'), findsOneWidget);
      expect(find.byType(CircularProgressIndicator), findsNothing);
      expect(
        tester
            .widget<Opacity>(
              find.byKey(const Key('skeleton-reveal-placeholder')),
            )
            .opacity,
        1,
      );

      relaySession.connect();
      await tester.pump();
      await tester.pump();
      expect(
        tester.widget<SkeletonReveal>(find.byType(SkeletonReveal)).loading,
        isFalse,
      );
      await tester.pump(const Duration(milliseconds: 200));

      expect(
        tester
            .widget<Opacity>(
              find.byKey(const Key('skeleton-reveal-placeholder')),
            )
            .opacity,
        closeTo(0.5, 0.01),
      );
      expect(
        tester
            .widget<Opacity>(find.byKey(const Key('skeleton-reveal-content')))
            .opacity,
        closeTo(0.5, 0.01),
      );

      await tester.pump(const Duration(milliseconds: 200));
      expect(
        tester
            .widget<Opacity>(find.byKey(const Key('skeleton-reveal-content')))
            .opacity,
        1,
      );
    });

    testWidgets('shows the first-load connection skeleton immediately', (
      tester,
    ) async {
      final relaySession = _ReconnectingRelaySession();
      await tester.pumpWidget(
        _buildTestable(
          messages: const [],
          messagesNotifier: _FakeMessagesNotifier(
            const [],
            hasLoadedMessages: false,
          ),
          relaySessionNotifier: relaySession,
        ),
      );
      await tester.pump();

      expect(
        tester.widget<SkeletonReveal>(find.byType(SkeletonReveal)).loading,
        isTrue,
      );
      expect(
        tester
            .widget<Opacity>(
              find.byKey(const Key('skeleton-reveal-placeholder')),
            )
            .opacity,
        1,
      );
      expect(
        tester
            .widget<Semantics>(
              find.byKey(const Key('channel-detail-connection-skeleton')),
            )
            .properties
            .label,
        'Reconnecting',
      );
    });

    testWidgets('keeps forum content visible with reconnect shimmer feedback', (
      tester,
    ) async {
      final relaySession = _ReconnectingRelaySession();
      final forumChannel = Channel(
        id: _channelId,
        name: 'design-forum',
        channelType: 'forum',
        visibility: 'open',
        description: 'Talk through design changes',
        createdBy: 'abc123',
        createdAt: DateTime(2025),
        memberCount: 5,
        isMember: true,
      );

      await tester.pumpWidget(
        _buildTestable(
          messages: const [],
          channel: forumChannel,
          relaySessionNotifier: relaySession,
        ),
      );
      await tester.pump();

      expect(find.byType(SkeletonReveal), findsNothing);
      expect(find.byKey(const Key('forum-connection-skeleton')), findsNothing);

      await tester.pump(const Duration(milliseconds: 1999));
      expect(find.byKey(const Key('forum-connection-skeleton')), findsNothing);

      await tester.pump(const Duration(milliseconds: 1));
      await tester.pump();
      final skeleton = find.byKey(const Key('forum-connection-skeleton'));
      expect(skeleton, findsOneWidget);
      expect(
        find.descendant(of: skeleton, matching: find.byType(SkeletonBar)),
        findsWidgets,
      );
      expect(find.byType(SkeletonReveal), findsNothing);
    });

    testWidgets('defers read-state mark until after build', (tester) async {
      final readState = _SynchronousReadStateNotifier(
        const ReadStateState(
          isReady: true,
          pubkey: 'self',
          contexts: {},
          version: 0,
        ),
      );

      await tester.pumpWidget(
        _buildTestable(
          messages: [
            _textMsg(
              id: 'msg1',
              pubkey: 'alice',
              content: 'First',
              createdAt: 1100,
            ),
            _textMsg(
              id: 'msg2',
              pubkey: 'alice',
              content: 'Latest',
              createdAt: 1200,
            ),
          ],
          readStateNotifier: readState,
        ),
      );

      expect(tester.takeException(), isNull);
      await tester.pump();

      expect(readState.markedContexts, {_channelId: 1200});
      expect(tester.takeException(), isNull);
    });

    testWidgets('shows forum posts view for forum channels', (tester) async {
      final forumChannel = Channel(
        id: _channelId,
        name: 'design-forum',
        channelType: 'forum',
        visibility: 'open',
        description: 'Talk through design changes',
        createdBy: 'abc123',
        createdAt: DateTime(2025),
        memberCount: 5,
        isMember: true,
      );

      await tester.pumpWidget(
        _buildTestable(messages: const [], channel: forumChannel),
      );
      // Allow the forum posts future provider to settle. It will error
      // because the stub relay has no real backend, but the ForumPostsView
      // should still render (showing an error or loading state).
      await tester.pump(const Duration(seconds: 1));

      // The old placeholder text should be gone.
      expect(find.text('Forum threads are not on mobile yet'), findsNothing);
      // The compose bar for stream messages should not appear.
      expect(find.text('Message…'), findsNothing);
    });

    testWidgets('renders video attachments from imeta tags in the timeline', (
      tester,
    ) async {
      const videoUrl = 'https://example.com/media/clip.mp4';

      await tester.pumpWidget(
        _buildTestable(
          messages: [
            _textMsg(
              id: 'video-1',
              pubkey: 'alice',
              content: '![video]($videoUrl)',
              extraTags: const [
                [
                  'imeta',
                  'url https://example.com/media/clip.mp4',
                  'm video/mp4',
                  'image https://example.com/media/poster.jpg',
                ],
              ],
            ),
          ],
          users: const {
            'alice': UserProfile(pubkey: 'alice', displayName: 'Alice'),
          },
        ),
      );
      await tester.pumpAndSettle();

      expect(
        find.byKey(
          const ValueKey(
            'message-media-video-preview:https://example.com/media/clip.mp4',
          ),
        ),
        findsOneWidget,
      );
    });

    testWidgets('members sheet shows roles and manage controls for owners', (
      tester,
    ) async {
      await tester.pumpWidget(
        _buildTestable(
          messages: const [],
          members: [
            ChannelMember(
              pubkey: 'self',
              role: 'owner',
              joinedAt: DateTime(2025),
              displayName: 'Self',
            ),
            ChannelMember(
              pubkey: 'alice',
              role: 'member',
              joinedAt: DateTime(2025),
              displayName: 'Alice',
            ),
          ],
        ),
      );
      await tester.pumpAndSettle();

      await tester.tap(find.byTooltip('View members'));
      await tester.pumpAndSettle();

      expect(find.text('Alice'), findsOneWidget);
      expect(find.text('Member'), findsOneWidget);
      expect(find.text('Owner'), findsOneWidget);
    });

    testWidgets('hides composer for archived channels', (tester) async {
      final archivedChannel = _testChannel.copyWith(
        archivedAt: DateTime.utc(2025, 1, 2),
      );

      await tester.pumpWidget(
        _buildTestable(messages: const [], channel: archivedChannel),
      );
      await tester.pumpAndSettle();

      expect(find.text('Message…'), findsNothing);
      expect(
        find.text('This channel is archived and read-only on mobile.'),
        findsOneWidget,
      );
    });

    testWidgets('updates detail page state after joining a channel', (
      tester,
    ) async {
      final openChannel = _testChannel.copyWith(isMember: false);
      final channelsNotifier = _FakeChannelsNotifier([openChannel]);

      await tester.pumpWidget(
        _buildTestable(
          messages: const [],
          channel: openChannel,
          channelsNotifier: channelsNotifier,
          createChannelActions: (ref) => _FakeChannelActions(
            ref,
            onJoinChannel: (_) async {
              channelsNotifier.setChannels([
                openChannel.copyWith(isMember: true, memberCount: 6),
              ]);
            },
          ),
        ),
      );
      await tester.pumpAndSettle();

      expect(
        find.text('Join this channel from Manage to participate.'),
        findsOneWidget,
      );
      expect(find.text('Message…'), findsNothing);

      await tester.tap(find.byTooltip('Manage channel'));
      await tester.pumpAndSettle();
      await tester.tap(find.text('Join channel'));
      await tester.pumpAndSettle();

      expect(find.text('Join channel'), findsNothing);
      expect(
        find.text('Join this channel from Manage to participate.'),
        findsNothing,
      );
      expect(find.text('Message #general'), findsOneWidget);
    });

    testWidgets('keeps manage sheet dismissible with a long canvas', (
      tester,
    ) async {
      tester.view.physicalSize = const Size(400, 800);
      tester.view.devicePixelRatio = 1;
      addTearDown(tester.view.resetPhysicalSize);
      addTearDown(tester.view.resetDevicePixelRatio);

      await tester.pumpWidget(
        _buildTestable(
          messages: const [],
          canvasContent: List.generate(
            80,
            (index) => 'Canvas line $index',
          ).join('\n'),
        ),
      );
      await tester.pumpAndSettle();

      await tester.tap(find.byTooltip('Manage channel'));
      await tester.pumpAndSettle();

      final sheet = find.byType(BottomSheet);
      expect(sheet, findsOneWidget);
      expect(tester.getSize(sheet).height, lessThanOrEqualTo(720));

      final sheetTop = tester.getTopLeft(sheet).dy;
      await tester.dragFrom(
        Offset(tester.view.physicalSize.width / 2, sheetTop + 12),
        const Offset(0, 800),
      );
      await tester.pumpAndSettle();

      expect(sheet, findsNothing);
    });

    testWidgets('shows empty state when no messages', (tester) async {
      await tester.pumpWidget(_buildTestable(messages: []));
      await tester.pumpAndSettle();

      expect(find.text('No messages yet'), findsOneWidget);
      expect(find.text('Be the first to say something!'), findsOneWidget);
    });

    testWidgets('renders text messages with author and content', (
      tester,
    ) async {
      final messages = [
        _textMsg(
          id: 'msg1',
          pubkey: 'alice',
          content: 'Hello world!',
          createdAt: 1000,
        ),
        _textMsg(
          id: 'msg2',
          pubkey: 'bob',
          content: 'Hey Alice!',
          createdAt: 1100,
        ),
      ];

      await tester.pumpWidget(
        _buildTestable(
          messages: messages,
          users: {
            'alice': const UserProfile(
              pubkey: 'alice',
              displayName: 'Alice',
              nip05Handle: 'alice@example.com',
            ),
            'bob': const UserProfile(pubkey: 'bob', displayName: 'Bob'),
          },
        ),
      );
      await tester.pumpAndSettle();

      expect(findRichText('Hello world!'), findsOneWidget);
      expect(findRichText('Hey Alice!'), findsOneWidget);
      expect(find.text('Alice'), findsOneWidget);
      expect(find.text('alice@example.com'), findsOneWidget);
      expect(find.text('Bob'), findsOneWidget);
      final messageAvatars = find.byType(CircleAvatar);
      expect(messageAvatars, findsNWidgets(2));
      for (final avatar in messageAvatars.evaluate()) {
        expect(
          tester.getSize(find.byWidget(avatar.widget)),
          const Size.square(messageAvatarSize),
        );
      }
      final aliceName = find.text('Alice');
      final aliceText = tester.widget<Text>(aliceName);
      expect(aliceText.style?.fontSize, messageUsernameTextStyle.fontSize);
      expect(aliceText.style?.fontWeight, messageUsernameTextStyle.fontWeight);
      expect(aliceText.style?.height, messageUsernameTextStyle.height);
      final aliceUsername = tester.widget<Text>(
        find.byKey(const ValueKey('message-username-msg1')),
      );
      final aliceTimestamp = tester.widget<Text>(
        find.byKey(const ValueKey('message-timestamp-msg1')),
      );
      expect(aliceUsername.style?.fontSize, messageMetadataTextStyle.fontSize);
      expect(aliceUsername.style?.fontWeight, FontWeight.w400);
      expect(aliceUsername.style?.height, messageMetadataTextStyle.height);
      expect(aliceTimestamp.style?.fontSize, messageMetadataTextStyle.fontSize);
      expect(aliceTimestamp.style?.fontWeight, FontWeight.w400);
      final helloContent = findRichText('Hello world!');
      final helloText = tester.widget<RichText>(helloContent);
      expect(
        effectiveFontSizeForText(helloText.text, 'Hello world!'),
        messageBodyTextStyle.fontSize,
      );
      final messageList = tester.widget<ScrollablePositionedList>(
        find.byKey(const ValueKey('channel-message-list')),
      );
      expect(messageList.padding!.bottom, 0);
      final newestMessageGroup = tester.widget<Padding>(
        find.byKey(const ValueKey('channel-message-group-msg2')),
      );
      expect(
        newestMessageGroup.padding,
        const EdgeInsets.only(bottom: Grid.xs),
      );
      expect(
        find.byKey(const ValueKey('channel-jump-to-latest')),
        findsNothing,
      );
      await tester.tap(find.text('Message #general'));
      await tester.pumpAndSettle();
      expect(
        find.byKey(const ValueKey('channel-jump-to-latest')),
        findsNothing,
      );
    });

    testWidgets('uses larger participant avatars in reply summaries', (
      tester,
    ) async {
      await tester.pumpWidget(
        _buildTestable(
          messages: [
            _textMsg(
              id: 'root',
              pubkey: 'alice',
              content: 'Thread head',
              createdAt: 1000,
            ),
            _textMsg(
              id: 'reply-1',
              pubkey: 'bob',
              content: 'First reply',
              createdAt: 1100,
              extraTags: const [
                ['e', 'root', '', 'reply'],
              ],
            ),
            _textMsg(
              id: 'reply-2',
              pubkey: 'carol',
              content: 'Second reply',
              createdAt: 1200,
              extraTags: const [
                ['e', 'root', '', 'reply'],
              ],
            ),
          ],
        ),
      );
      await tester.pumpAndSettle();

      expect(
        findRichText(
          '2 replies · last reply '
          '${formatThreadSummaryLastReplyTime(1200)}',
        ),
        findsOneWidget,
      );
      expect(find.byIcon(LucideIcons.chevronRight), findsNothing);
      final replyAvatars = find.byType(SmallAvatar);
      expect(replyAvatars, findsNWidgets(2));
      for (final avatar in replyAvatars.evaluate()) {
        expect(
          tester.getSize(find.byWidget(avatar.widget)),
          const Size.square(32),
        );
      }
      final summaryPadding = tester.widget<Padding>(
        find.byKey(const ValueKey('thread-summary-root')),
      );
      expect(
        summaryPadding.padding,
        const EdgeInsets.only(
          left: messageAvatarSize + messageAvatarContentGap,
          top: Grid.half,
          bottom: Grid.xs,
        ),
      );
    });

    testWidgets('constrains reply summaries at accessibility text sizes', (
      tester,
    ) async {
      tester.view.physicalSize = const Size(400, 800);
      tester.view.devicePixelRatio = 1;
      addTearDown(tester.view.resetPhysicalSize);
      addTearDown(tester.view.resetDevicePixelRatio);

      final lastReplyAt =
          DateTime.now().millisecondsSinceEpoch ~/ 1000 - 59 * 60;
      await tester.pumpWidget(
        _buildTestable(
          messages: [
            _textMsg(
              id: 'root',
              pubkey: 'alice',
              content: 'Thread head',
              createdAt: lastReplyAt - 300,
            ),
            for (var i = 0; i < 3; i++)
              _textMsg(
                id: 'reply-$i',
                pubkey: 'participant-$i',
                content: 'Reply $i',
                createdAt: lastReplyAt - 2 + i,
                extraTags: const [
                  ['e', 'root', '', 'reply'],
                ],
              ),
          ],
          channel: _testChannel.copyWith(archivedAt: DateTime.now()),
          textScaler: const TextScaler.linear(2),
        ),
      );
      await tester.pumpAndSettle();

      final summaryText = tester.widget<RichText>(findRichText('3 replies'));
      expect(summaryText.maxLines, 2);
      expect(summaryText.overflow, TextOverflow.ellipsis);
      expect(tester.takeException(), isNull);
    });

    testWidgets('can jump back to latest after a non-drag user scroll', (
      tester,
    ) async {
      final initialMessages = [
        for (var i = 0; i < 40; i++)
          _textMsg(
            id: 'msg$i',
            pubkey: 'alice',
            content: 'Message $i',
            createdAt: 1000 + i,
          ),
      ];
      final messagesNotifier = _FakeMessagesNotifier(initialMessages);

      await tester.pumpWidget(
        _buildTestable(
          messages: const [],
          messagesNotifier: messagesNotifier,
          users: const {
            'alice': UserProfile(pubkey: 'alice', displayName: 'Alice'),
          },
        ),
      );
      await tester.pumpAndSettle();

      final messageList = find.byKey(const ValueKey('channel-message-list'));
      final messageListElement = tester.element(messageList);
      UserScrollNotification(
        metrics: FixedScrollMetrics(
          minScrollExtent: 0,
          maxScrollExtent: 100,
          pixels: 0,
          viewportDimension: 100,
          axisDirection: AxisDirection.down,
          devicePixelRatio: 1,
        ),
        context: messageListElement,
        direction: ScrollDirection.reverse,
      ).dispatch(messageListElement);
      final listView = tester.widget<ScrollablePositionedList>(messageList);
      listView.itemScrollController!.jumpTo(index: 39);
      await tester.pumpAndSettle();
      expect(
        find.byKey(const ValueKey('channel-jump-to-latest')),
        findsOneWidget,
      );

      messagesNotifier.setMessages([
        ...initialMessages,
        _textMsg(
          id: 'newest',
          pubkey: 'alice',
          content: 'Newest live update',
          createdAt: 2000,
        ),
      ]);
      await tester.pump();

      expect(findRichText('Newest live update'), findsNothing);
      await tester.tap(find.byKey(const ValueKey('channel-jump-to-latest')));
      await tester.pumpAndSettle();

      expect(findRichText('Newest live update'), findsOneWidget);
    });

    testWidgets(
      'keeps follow mode off while a tall newest message stays visible',
      (tester) async {
        tester.view.physicalSize = const Size(400, 600);
        tester.view.devicePixelRatio = 1;
        addTearDown(tester.view.resetPhysicalSize);
        addTearDown(tester.view.resetDevicePixelRatio);

        final tallMessage = List.generate(
          12,
          (index) => 'Newest message line $index',
        ).join('\n');
        final initialMessages = [
          for (var i = 0; i < 12; i++)
            _textMsg(
              id: 'msg$i',
              pubkey: i.isEven ? 'alice' : 'bob',
              content: 'Message $i',
              createdAt: 1000 + i * 1000,
            ),
          _textMsg(
            id: 'tall-newest',
            pubkey: 'alice',
            content: tallMessage,
            createdAt: 20_000,
          ),
        ];
        final messagesNotifier = _FakeMessagesNotifier(initialMessages);

        await tester.pumpWidget(
          _buildTestable(
            messages: const [],
            messagesNotifier: messagesNotifier,
            users: const {
              'alice': UserProfile(pubkey: 'alice', displayName: 'Alice'),
              'bob': UserProfile(pubkey: 'bob', displayName: 'Bob'),
            },
          ),
        );
        await tester.pumpAndSettle();

        final messageList = find.byKey(const ValueKey('channel-message-list'));
        await tester.drag(messageList, const Offset(0, 120));
        await tester.pumpAndSettle();

        expect(findRichText('Newest message line 0'), findsOneWidget);
        expect(
          find.byKey(const ValueKey('channel-jump-to-latest')),
          findsOneWidget,
        );

        messagesNotifier.setMessages([
          ...initialMessages,
          _textMsg(
            id: 'newest-live',
            pubkey: 'alice',
            content: 'Newest live update',
            createdAt: 30_000,
          ),
        ]);
        await tester.pumpAndSettle();

        expect(findRichText('Newest live update'), findsNothing);
        expect(
          find.byKey(const ValueKey('channel-jump-to-latest')),
          findsOneWidget,
        );
      },
    );

    testWidgets('preserves an initial message deep-link position', (
      tester,
    ) async {
      final initialMessages = [
        for (var i = 0; i < 40; i++)
          _textMsg(
            id: 'msg$i',
            pubkey: 'alice',
            content: 'Message $i',
            createdAt: 1000 + i,
          ),
      ];
      final messagesNotifier = _FakeMessagesNotifier(initialMessages);

      await tester.pumpWidget(
        _buildTestable(
          messages: const [],
          messagesNotifier: messagesNotifier,
          initialMessageId: 'msg5',
          users: const {
            'alice': UserProfile(pubkey: 'alice', displayName: 'Alice'),
          },
        ),
      );
      await tester.pumpAndSettle();

      expect(findRichText('Message 5'), findsOneWidget);
      expect(findRichText('Message 39'), findsNothing);
      expect(
        find.byKey(const ValueKey('channel-jump-to-latest')),
        findsOneWidget,
      );

      messagesNotifier.setMessages([
        ...initialMessages,
        _textMsg(
          id: 'newest',
          pubkey: 'alice',
          content: 'Newest live update',
          createdAt: 2000,
        ),
      ]);
      await tester.pumpAndSettle();

      expect(findRichText('Message 5'), findsOneWidget);
      expect(findRichText('Newest live update'), findsNothing);
    });

    testWidgets(
      'keeps a deep-linked message in view when its page arrives after a '
      'small scroll near the latest message',
      (tester) async {
        tester.view.physicalSize = const Size(400, 800);
        tester.view.devicePixelRatio = 1;
        addTearDown(tester.view.resetPhysicalSize);
        addTearDown(tester.view.resetDevicePixelRatio);

        // The deep-link target lives in an older page that has not loaded yet.
        final messagesNotifier = _FakeMessagesNotifier([
          for (var i = 30; i < 60; i++)
            _textMsg(
              id: 'msg$i',
              pubkey: 'alice',
              content: 'Message $i',
              createdAt: 1000 + i * 1000,
            ),
        ]);

        await tester.pumpWidget(
          _buildTestable(
            messages: const [],
            messagesNotifier: messagesNotifier,
            initialMessageId: 'msg3',
            users: const {
              'alice': UserProfile(pubkey: 'alice', displayName: 'Alice'),
            },
          ),
        );
        await tester.pumpAndSettle();

        // Small scrolls that keep the newest message visible, so isAtLatest
        // stays true while the scroll offset becomes non-zero. This lets a
        // later programmatic jumpTo dispatch ScrollEndNotification.
        for (final dy in const [10.0, 20.0, 30.0]) {
          await tester.drag(
            find.byKey(const ValueKey('channel-message-list')),
            Offset(0, dy),
          );
          await tester.pumpAndSettle();
        }

        // The older page containing the deep-link target arrives.
        messagesNotifier.setMessages([
          for (var i = 0; i < 60; i++)
            _textMsg(
              id: 'msg$i',
              pubkey: 'alice',
              content: 'Message $i',
              createdAt: 1000 + i * 1000,
            ),
        ]);
        await tester.pumpAndSettle();

        // The deep-link jump must stick rather than snapping back to newest.
        expect(findRichText('Message 3'), findsOneWidget);
        expect(findRichText('Message 59'), findsNothing);
      },
    );

    testWidgets('groups consecutive messages from same author', (tester) async {
      final messages = [
        _textMsg(
          id: 'msg1',
          pubkey: 'alice',
          content: 'First message',
          createdAt: 1000,
        ),
        _textMsg(
          id: 'msg2',
          pubkey: 'alice',
          content: 'Second message',
          createdAt: 1060, // within 5 min
        ),
      ];

      await tester.pumpWidget(
        _buildTestable(
          messages: messages,
          users: {
            'alice': const UserProfile(pubkey: 'alice', displayName: 'Alice'),
          },
        ),
      );
      await tester.pumpAndSettle();

      // Author name should appear only once (grouped).
      expect(find.text('Alice'), findsOneWidget);
      expect(findRichText('First message'), findsOneWidget);
      expect(findRichText('Second message'), findsOneWidget);
    });

    testWidgets('shows author again after 5min gap', (tester) async {
      final messages = [
        _textMsg(
          id: 'msg1',
          pubkey: 'alice',
          content: 'First',
          createdAt: 1000,
        ),
        _textMsg(
          id: 'msg2',
          pubkey: 'alice',
          content: 'Second',
          createdAt: 1400, // 6+ min later
        ),
      ];

      await tester.pumpWidget(
        _buildTestable(
          messages: messages,
          users: {
            'alice': const UserProfile(pubkey: 'alice', displayName: 'Alice'),
          },
        ),
      );
      await tester.pumpAndSettle();

      // Author name appears twice since messages are >5min apart.
      expect(find.text('Alice'), findsNWidgets(2));
    });

    testWidgets('shows pubkey fallback when no profile', (tester) async {
      final messages = [
        _textMsg(
          id: 'msg1',
          pubkey: 'abcdef1234567890',
          content: 'Hi',
          createdAt: 1000,
        ),
      ];

      await tester.pumpWidget(_buildTestable(messages: messages));
      await tester.pumpAndSettle();

      expect(findRichText('Hi'), findsOneWidget);
      // Should show first 8 chars of pubkey + ellipsis
      expect(find.text('abcdef12…'), findsOneWidget);
    });
  });

  group('System messages', () {
    testWidgets('renders channel_created system event', (tester) async {
      final messages = [
        _systemMsg(
          id: 'sys1',
          payload: {'type': 'channel_created', 'actor': 'alice'},
        ),
      ];

      await tester.pumpWidget(
        _buildTestable(
          messages: messages,
          users: {
            'alice': const UserProfile(pubkey: 'alice', displayName: 'Alice'),
          },
        ),
      );
      await tester.pumpAndSettle();

      expect(find.text('Alice'), findsOneWidget);
      final createdAction = findRichText('created this channel');
      expect(createdAction, findsOneWidget);
      expect(
        tester.getSize(find.byType(CircleAvatar)),
        const Size.square(messageAvatarSize),
      );
      final nameRect = tester.getRect(find.text('Alice'));
      final nameText = tester.widget<Text>(find.text('Alice'));
      expect(nameText.style?.fontSize, systemMessageHeadingTextStyle.fontSize);
      expect(
        nameText.style?.fontWeight,
        systemMessageHeadingTextStyle.fontWeight,
      );
      expect(
        find.byKey(const ValueKey('system-message-username-alice')),
        findsNothing,
      );
      final timestampRect = tester.getRect(
        find.byKey(const ValueKey('system-message-timestamp-alice')),
      );
      expect(timestampRect.left, greaterThan(nameRect.right));
      final createdText = tester.widget<RichText>(createdAction);
      expect(
        effectiveFontSizeForText(createdText.text, 'created this channel'),
        systemMessageBodyTextStyle.fontSize,
      );
    });

    testWidgets('renders member_joined (self-join) system event', (
      tester,
    ) async {
      final messages = [
        _systemMsg(
          id: 'sys1',
          payload: {'type': 'member_joined', 'actor': 'bob', 'target': 'bob'},
        ),
      ];

      await tester.pumpWidget(
        _buildTestable(
          messages: messages,
          users: {'bob': const UserProfile(pubkey: 'bob', displayName: 'Bob')},
        ),
      );
      await tester.pumpAndSettle();

      expect(find.text('Bob'), findsOneWidget);
      expect(findRichText('joined the channel'), findsOneWidget);
      expect(
        tester.getSize(find.byType(CircleAvatar)),
        const Size.square(messageAvatarSize),
      );
    });

    testWidgets('renders member_joined (added by other) system event', (
      tester,
    ) async {
      final messages = [
        _systemMsg(
          id: 'sys1',
          payload: {'type': 'member_joined', 'actor': 'alice', 'target': 'bob'},
        ),
      ];

      await tester.pumpWidget(
        _buildTestable(
          messages: messages,
          users: {
            'alice': const UserProfile(pubkey: 'alice', displayName: 'Alice'),
            'bob': const UserProfile(pubkey: 'bob', displayName: 'Bob'),
          },
        ),
      );
      await tester.pumpAndSettle();

      expect(find.text('Bob'), findsOneWidget);
      final addedAction = findRichText('was added by Alice');
      expect(addedAction, findsOneWidget);
      expect(find.text('Alice added Bob to the channel'), findsNothing);
      expect(
        tester.getSize(find.byType(CircleAvatar)),
        const Size.square(messageAvatarSize),
      );
      final nameRect = tester.getRect(find.text('Bob'));
      expect(
        find.byKey(const ValueKey('system-message-username-bob')),
        findsNothing,
      );
      final timestampRect = tester.getRect(
        find.byKey(const ValueKey('system-message-timestamp-bob')),
      );
      expect(timestampRect.left, greaterThan(nameRect.right));
      final addedText = tester.widget<RichText>(addedAction);
      expect(
        effectiveFontSizeForText(addedText.text, 'was added by Alice'),
        systemMessageBodyTextStyle.fontSize,
      );
    });

    testWidgets('groups member additions with tappable overflow names', (
      tester,
    ) async {
      final messages = [
        _systemMsg(
          id: 'sys1',
          payload: {'type': 'member_joined', 'actor': 'alice', 'target': 'bob'},
          createdAt: 1000,
        ),
        _systemMsg(
          id: 'sys2',
          payload: {
            'type': 'member_joined',
            'actor': 'alice',
            'target': 'carol',
          },
          createdAt: 1060,
        ),
        _systemMsg(
          id: 'sys3',
          payload: {
            'type': 'member_joined',
            'actor': 'alice',
            'target': 'dave',
          },
          createdAt: 1120,
        ),
        _systemMsg(
          id: 'sys4',
          payload: {
            'type': 'member_joined',
            'actor': 'alice',
            'target': 'erin',
          },
          createdAt: 1180,
        ),
        _systemMsg(
          id: 'sys5',
          payload: {
            'type': 'member_joined',
            'actor': 'alice',
            'target': 'frank',
          },
          createdAt: 1240,
        ),
      ];

      await tester.pumpWidget(
        _buildTestable(
          messages: messages,
          users: {
            'alice': const UserProfile(pubkey: 'alice', displayName: 'Alice'),
            'bob': const UserProfile(pubkey: 'bob', displayName: 'Bob'),
            'carol': const UserProfile(pubkey: 'carol', displayName: 'Carol'),
            'dave': const UserProfile(pubkey: 'dave', displayName: 'Dave'),
            'erin': const UserProfile(pubkey: 'erin', displayName: 'Erin'),
            'frank': const UserProfile(pubkey: 'frank', displayName: 'Frank'),
          },
        ),
      );
      await tester.pumpAndSettle();

      expect(find.text('Bob'), findsOneWidget);
      expect(
        findRichText('was added by Alice, along with Carol, Dave, Erin, and '),
        findsOneWidget,
      );
      expect(find.byKey(const Key('membership-overflow')), findsOneWidget);
      expect(find.text('1 others'), findsOneWidget);
      expect(find.byTooltip('Frank'), findsOneWidget);
    });

    testWidgets('aligns grouped reactions with the system message content', (
      tester,
    ) async {
      final messages = [
        _systemMsg(
          id: 'sys1',
          payload: {'type': 'member_joined', 'actor': 'alice', 'target': 'bob'},
          createdAt: 1000,
        ),
        _systemMsg(
          id: 'sys2',
          payload: {
            'type': 'member_joined',
            'actor': 'alice',
            'target': 'carol',
          },
          createdAt: 1060,
        ),
        _reaction(id: 'reaction-1', targetId: 'sys1'),
      ];

      await tester.pumpWidget(
        _buildTestable(
          messages: messages,
          users: {
            'alice': const UserProfile(pubkey: 'alice', displayName: 'Alice'),
            'bob': const UserProfile(pubkey: 'bob', displayName: 'Bob'),
            'carol': const UserProfile(pubkey: 'carol', displayName: 'Carol'),
          },
        ),
      );
      await tester.pumpAndSettle();

      final avatarRect = tester.getRect(find.byType(CircleAvatar));
      final reactionRect = tester.getRect(find.byType(ReactionRow));
      expect(
        reactionRect.left,
        avatarRect.left + messageAvatarSize + messageAvatarContentGap,
      );
    });

    testWidgets('renders member_left system event', (tester) async {
      final messages = [
        _systemMsg(
          id: 'sys1',
          payload: {'type': 'member_left', 'actor': 'bob'},
        ),
      ];

      await tester.pumpWidget(
        _buildTestable(
          messages: messages,
          users: {'bob': const UserProfile(pubkey: 'bob', displayName: 'Bob')},
        ),
      );
      await tester.pumpAndSettle();

      expect(find.text('Bob left the channel'), findsOneWidget);
    });

    testWidgets(
      'constrains generic system timestamps at accessibility text sizes',
      (tester) async {
        tester.view.physicalSize = const Size(240, 600);
        tester.view.devicePixelRatio = 1;
        addTearDown(tester.view.resetPhysicalSize);
        addTearDown(tester.view.resetDevicePixelRatio);

        await tester.pumpWidget(
          _buildTestable(
            messages: [
              _systemMsg(
                id: 'sys-accessible',
                payload: {
                  'type': 'topic_changed',
                  'actor': 'alice',
                  'topic': 'Release planning',
                },
                createdAt:
                    DateTime(2026, 7, 28, 12, 34).millisecondsSinceEpoch ~/
                    1000,
              ),
            ],
            users: {
              'alice': const UserProfile(pubkey: 'alice', displayName: 'Alice'),
            },
            textScaler: const TextScaler.linear(3),
          ),
        );
        await tester.pumpAndSettle();

        final timestampFinder = find.byKey(
          const ValueKey('system-message-timestamp-sys-accessible'),
        );
        final timestamp = tester.widget<Text>(timestampFinder);
        expect(timestamp.maxLines, 1);
        expect(timestamp.overflow, TextOverflow.ellipsis);
        expect(
          tester.getSize(timestampFinder).width,
          lessThanOrEqualTo(Grid.xxl),
        );
        expect(tester.takeException(), isNull);
      },
    );

    testWidgets('renders member_removed system event', (tester) async {
      final messages = [
        _systemMsg(
          id: 'sys1',
          payload: {
            'type': 'member_removed',
            'actor': 'alice',
            'target': 'bob',
          },
        ),
      ];

      await tester.pumpWidget(
        _buildTestable(
          messages: messages,
          users: {
            'alice': const UserProfile(pubkey: 'alice', displayName: 'Alice'),
            'bob': const UserProfile(pubkey: 'bob', displayName: 'Bob'),
          },
        ),
      );
      await tester.pumpAndSettle();

      expect(find.text('Alice removed Bob from the channel'), findsOneWidget);
    });

    testWidgets('renders topic_changed system event', (tester) async {
      final messages = [
        _systemMsg(
          id: 'sys1',
          payload: {
            'type': 'topic_changed',
            'actor': 'alice',
            'topic': 'Release planning',
          },
        ),
      ];

      await tester.pumpWidget(
        _buildTestable(
          messages: messages,
          users: {
            'alice': const UserProfile(pubkey: 'alice', displayName: 'Alice'),
          },
        ),
      );
      await tester.pumpAndSettle();

      expect(
        find.text('Alice changed the topic to "Release planning"'),
        findsOneWidget,
      );
    });

    testWidgets('renders purpose_changed system event', (tester) async {
      final messages = [
        _systemMsg(
          id: 'sys1',
          payload: {
            'type': 'purpose_changed',
            'actor': 'alice',
            'purpose': 'Team standup notes',
          },
        ),
      ];

      await tester.pumpWidget(
        _buildTestable(
          messages: messages,
          users: {
            'alice': const UserProfile(pubkey: 'alice', displayName: 'Alice'),
          },
        ),
      );
      await tester.pumpAndSettle();

      expect(
        find.text('Alice changed the purpose to "Team standup notes"'),
        findsOneWidget,
      );
    });

    testWidgets('system message breaks author grouping', (tester) async {
      final messages = [
        _textMsg(
          id: 'msg1',
          pubkey: 'alice',
          content: 'Before',
          createdAt: 1000,
        ),
        _systemMsg(
          id: 'sys1',
          payload: {'type': 'member_joined', 'actor': 'bob', 'target': 'bob'},
          createdAt: 1010,
        ),
        _textMsg(
          id: 'msg2',
          pubkey: 'alice',
          content: 'After',
          createdAt: 1020,
        ),
      ];

      await tester.pumpWidget(
        _buildTestable(
          messages: messages,
          users: {
            'alice': const UserProfile(pubkey: 'alice', displayName: 'Alice'),
            'bob': const UserProfile(pubkey: 'bob', displayName: 'Bob'),
          },
        ),
      );
      await tester.pumpAndSettle();

      // Alice should appear twice — system message breaks grouping.
      expect(find.text('Alice'), findsNWidgets(2));
    });

    testWidgets('skips unknown system event types', (tester) async {
      final messages = [
        _systemMsg(
          id: 'sys1',
          payload: {'type': 'unknown_future_type', 'actor': 'alice'},
        ),
        _textMsg(
          id: 'msg1',
          pubkey: 'alice',
          content: 'Hello',
          createdAt: 1100,
        ),
      ];

      await tester.pumpWidget(
        _buildTestable(
          messages: messages,
          users: {
            'alice': const UserProfile(pubkey: 'alice', displayName: 'Alice'),
          },
        ),
      );
      await tester.pumpAndSettle();

      // Only the text message should render, unknown system event is skipped.
      expect(findRichText('Hello'), findsOneWidget);
      // No system message row rendered for unknown type.
      expect(find.byIcon(LucideIcons.arrowLeftRight), findsNothing);
    });
  });

  group('Deletions', () {
    testWidgets('deleted messages are not shown', (tester) async {
      final messages = [
        _textMsg(
          id: 'msg1',
          pubkey: 'alice',
          content: 'Keep this',
          createdAt: 1000,
        ),
        _textMsg(
          id: 'msg2',
          pubkey: 'bob',
          content: 'Delete this',
          createdAt: 1100,
        ),
        _deletion(id: 'del1', targetIds: ['msg2'], createdAt: 1200),
      ];

      await tester.pumpWidget(
        _buildTestable(
          messages: messages,
          users: {
            'alice': const UserProfile(pubkey: 'alice', displayName: 'Alice'),
            'bob': const UserProfile(pubkey: 'bob', displayName: 'Bob'),
          },
        ),
      );
      await tester.pumpAndSettle();

      expect(findRichText('Keep this'), findsOneWidget);
      expect(findRichText('Delete this'), findsNothing);
    });

    testWidgets('deletion of multiple messages', (tester) async {
      final messages = [
        _textMsg(id: 'msg1', pubkey: 'a', content: 'One', createdAt: 1000),
        _textMsg(id: 'msg2', pubkey: 'a', content: 'Two', createdAt: 1100),
        _textMsg(id: 'msg3', pubkey: 'a', content: 'Three', createdAt: 1200),
        _deletion(id: 'del1', targetIds: ['msg1', 'msg3'], createdAt: 1300),
      ];

      await tester.pumpWidget(_buildTestable(messages: messages));
      await tester.pumpAndSettle();

      expect(findRichText('One'), findsNothing);
      expect(findRichText('Two'), findsOneWidget);
      expect(findRichText('Three'), findsNothing);
    });
  });

  group('Edits', () {
    testWidgets('edited message shows updated content and (edited) label', (
      tester,
    ) async {
      final messages = [
        _textMsg(
          id: 'msg1',
          pubkey: 'alice',
          content: 'Original text',
          createdAt: 1000,
        ),
        _edit(
          id: 'edit1',
          targetId: 'msg1',
          content: 'Edited text',
          createdAt: 1100,
        ),
      ];

      await tester.pumpWidget(
        _buildTestable(
          messages: messages,
          users: {
            'alice': const UserProfile(pubkey: 'alice', displayName: 'Alice'),
          },
        ),
      );
      await tester.pumpAndSettle();

      expect(findRichText('Edited text'), findsOneWidget);
      expect(findRichText('Original text'), findsNothing);
      expect(find.text('(edited)'), findsOneWidget);
    });

    testWidgets('latest edit wins when multiple edits exist', (tester) async {
      final messages = [
        _textMsg(id: 'msg1', pubkey: 'alice', content: 'V1', createdAt: 1000),
        _edit(id: 'e1', targetId: 'msg1', content: 'V2', createdAt: 1100),
        _edit(id: 'e2', targetId: 'msg1', content: 'V3', createdAt: 1200),
      ];

      await tester.pumpWidget(
        _buildTestable(
          messages: messages,
          users: {
            'alice': const UserProfile(pubkey: 'alice', displayName: 'Alice'),
          },
        ),
      );
      await tester.pumpAndSettle();

      expect(findRichText('V3'), findsOneWidget);
      expect(findRichText('V1'), findsNothing);
      expect(findRichText('V2'), findsNothing);
    });
  });

  group('Typing indicator', () {
    testWidgets('shows single typer', (tester) async {
      await tester.pumpWidget(
        _buildTestable(
          messages: [],
          typing: [
            TypingEntry(
              pubkey: 'alice',
              expiresAtMs: DateTime.now().millisecondsSinceEpoch + 8000,
            ),
          ],
          users: {
            'alice': const UserProfile(pubkey: 'alice', displayName: 'Alice'),
          },
        ),
      );
      await tester.pumpAndSettle();

      expect(find.text('Alice is typing…'), findsOneWidget);
    });

    testWidgets('shows two typers', (tester) async {
      await tester.pumpWidget(
        _buildTestable(
          messages: [],
          typing: [
            TypingEntry(
              pubkey: 'alice',
              expiresAtMs: DateTime.now().millisecondsSinceEpoch + 8000,
            ),
            TypingEntry(
              pubkey: 'bob',
              expiresAtMs: DateTime.now().millisecondsSinceEpoch + 8000,
            ),
          ],
          users: {
            'alice': const UserProfile(pubkey: 'alice', displayName: 'Alice'),
            'bob': const UserProfile(pubkey: 'bob', displayName: 'Bob'),
          },
        ),
      );
      await tester.pumpAndSettle();

      expect(find.text('Alice and Bob are typing…'), findsOneWidget);
    });

    testWidgets('shows N others for 3+ typers', (tester) async {
      await tester.pumpWidget(
        _buildTestable(
          messages: [],
          typing: [
            TypingEntry(
              pubkey: 'alice',
              expiresAtMs: DateTime.now().millisecondsSinceEpoch + 8000,
            ),
            TypingEntry(
              pubkey: 'bob',
              expiresAtMs: DateTime.now().millisecondsSinceEpoch + 8000,
            ),
            TypingEntry(
              pubkey: 'carol',
              expiresAtMs: DateTime.now().millisecondsSinceEpoch + 8000,
            ),
          ],
          users: {
            'alice': const UserProfile(pubkey: 'alice', displayName: 'Alice'),
            'bob': const UserProfile(pubkey: 'bob', displayName: 'Bob'),
            'carol': const UserProfile(pubkey: 'carol', displayName: 'Carol'),
          },
        ),
      );
      await tester.pumpAndSettle();

      expect(find.text('Alice and 2 others are typing…'), findsOneWidget);
    });
  });

  group('Compose bar', () {
    testWidgets('expands from the channel hint into the composer controls', (
      tester,
    ) async {
      await tester.pumpWidget(_buildTestable(messages: []));
      await tester.pumpAndSettle();

      expect(find.byType(TextField), findsNothing);
      expect(find.byIcon(LucideIcons.arrowUp).hitTestable(), findsNothing);

      await tester.tap(find.text('Message #general'));
      await tester.pumpAndSettle();

      expect(find.byType(TextField), findsOneWidget);
      expect(find.byIcon(LucideIcons.arrowUp).hitTestable(), findsOneWidget);
    });

    testWidgets('shows hint text', (tester) async {
      await tester.pumpWidget(_buildTestable(messages: []));
      await tester.pumpAndSettle();

      expect(find.text('Message #general'), findsOneWidget);
    });
  });

  group('App bar', () {
    testWidgets('shows channel name with hash icon', (tester) async {
      await tester.pumpWidget(_buildTestable(messages: []));
      await tester.pumpAndSettle();

      expect(find.text('general'), findsOneWidget);
      // The hash icon appears in the app bar and in the compose bar toolbar.
      expect(find.byIcon(LucideIcons.hash), findsAtLeastNWidgets(1));
    });

    testWidgets('shows lock icon for private channel', (tester) async {
      final privateChannel = Channel(
        id: _channelId,
        name: 'secret',
        channelType: 'stream',
        visibility: 'private',
        description: 'Private channel',
        createdBy: 'abc',
        createdAt: DateTime(2025),
        memberCount: 3,
        isMember: true,
      );

      await tester.pumpWidget(
        _buildTestable(messages: [], channel: privateChannel),
      );
      await tester.pumpAndSettle();

      expect(find.text('secret'), findsOneWidget);
      expect(find.byIcon(LucideIcons.lock), findsOneWidget);
    });
  });

  group('Error and loading states', () {
    testWidgets('shows error message on failure', (tester) async {
      await tester.pumpWidget(
        ProviderScope(
          overrides: [
            channelMessagesProvider(
              _channelId,
            ).overrideWith(() => _ErrorMessagesNotifier()),
            channelTypingProvider(
              _channelId,
            ).overrideWith(() => _FakeTypingNotifier([])),
            userCacheProvider.overrideWith(() => _FakeUserCacheNotifier({})),
            channelsProvider.overrideWith(
              () => _FakeChannelsNotifier([_testChannel]),
            ),
            relayClientProvider.overrideWithValue(
              RelayClient(baseUrl: 'http://localhost:3000'),
            ),
            savedPrefsProvider.overrideWithValue(_testPrefs),
          ],
          child: MaterialApp(
            theme: AppTheme.light(),
            home: ChannelDetailPage(channel: _testChannel),
          ),
        ),
      );
      await tester.pumpAndSettle();

      expect(find.text('Failed to load messages'), findsOneWidget);
    });
  });

  group('Mixed message timeline', () {
    testWidgets('interleaves text and system messages correctly', (
      tester,
    ) async {
      final messages = [
        _systemMsg(
          id: 'sys1',
          payload: {'type': 'channel_created', 'actor': 'alice'},
          createdAt: 900,
        ),
        _textMsg(
          id: 'msg1',
          pubkey: 'alice',
          content: 'Welcome everyone!',
          createdAt: 1000,
        ),
        _systemMsg(
          id: 'sys2',
          payload: {'type': 'member_joined', 'actor': 'bob', 'target': 'bob'},
          createdAt: 1100,
        ),
        _textMsg(
          id: 'msg2',
          pubkey: 'bob',
          content: 'Thanks for the invite!',
          createdAt: 1200,
        ),
      ];

      await tester.pumpWidget(
        _buildTestable(
          messages: messages,
          users: {
            'alice': const UserProfile(pubkey: 'alice', displayName: 'Alice'),
            'bob': const UserProfile(pubkey: 'bob', displayName: 'Bob'),
          },
        ),
      );
      await tester.pumpAndSettle();

      expect(find.text('Alice'), findsNWidgets(2));
      expect(findRichText('created this channel'), findsOneWidget);
      expect(findRichText('Welcome everyone!'), findsOneWidget);
      expect(find.text('Bob'), findsNWidgets(2));
      expect(findRichText('joined the channel'), findsOneWidget);
      expect(findRichText('Thanks for the invite!'), findsOneWidget);
    });
  });

  group('Deep-link navigation', () {
    testWidgets('opens a nested reply in its direct-parent thread', (
      tester,
    ) async {
      final root = _textMsg(
        id: 'root',
        pubkey: 'alice',
        content: 'Outer root',
        createdAt: 1000,
      );
      final parent = _textMsg(
        id: 'parent',
        pubkey: 'bob',
        content: 'Nested thread head',
        createdAt: 1100,
        extraTags: const [
          ['e', 'root', '', 'reply'],
        ],
      );
      final target = _textMsg(
        id: 'target',
        pubkey: 'carol',
        content: 'Deeply nested target',
        createdAt: 1200,
        extraTags: const [
          ['e', 'root', '', 'root'],
          ['e', 'parent', '', 'reply'],
        ],
      );

      await tester.pumpWidget(
        _buildTestable(
          messages: [root, parent, target],
          initialMessageId: 'target',
          initialThreadRootId: 'parent',
          threadReplies: {
            // Relay subtree filtering is keyed by thread_metadata.root_event_id,
            // so nested replies are returned by the outer-root query.
            'root': [parent, target],
          },
          users: const {
            'alice': UserProfile(pubkey: 'alice', displayName: 'Alice'),
            'bob': UserProfile(pubkey: 'bob', displayName: 'Bob'),
            'carol': UserProfile(pubkey: 'carol', displayName: 'Carol'),
          },
        ),
      );
      await tester.pumpAndSettle();

      final threadPage = tester.widget<ThreadDetailPage>(
        find.byType(ThreadDetailPage),
      );
      expect(threadPage.threadHead.id, 'parent');
      expect(threadPage.initialMessageId, 'target');

      final highlighted = tester.widget<DecoratedBox>(
        find.byKey(const ValueKey('thread-message-target')),
      );
      final decoration = highlighted.decoration as BoxDecoration;
      expect(decoration.color, isNot(Colors.transparent));
    });
  });

  group('Channel links', () {
    testWidgets('tapping a channel link opens that channel', (tester) async {
      final randomChannel = _channel(id: 'random-channel', name: 'random');
      final observer = _TestNavigatorObserver();

      await tester.pumpWidget(
        _buildTestable(
          messages: [
            _textMsg(
              id: 'msg1',
              pubkey: 'alice',
              content: 'Take this to #random',
              createdAt: 1000,
            ),
          ],
          users: {
            'alice': const UserProfile(pubkey: 'alice', displayName: 'Alice'),
          },
          channels: [_testChannel, randomChannel],
          navigatorObservers: [observer],
        ),
      );
      await tester.pumpAndSettle();
      final initialPushCount = observer.pushCount;

      await tester.tap(find.text('#random'));
      await tester.pumpAndSettle();

      expect(observer.pushCount, initialPushCount + 1);
    });

    testWidgets('missing channel link shows an error', (tester) async {
      final randomChannel = _channel(id: 'random-channel', name: 'random');
      final channelsNotifier = _FakeChannelsNotifier([
        _testChannel,
        randomChannel,
      ]);

      await tester.pumpWidget(
        _buildTestable(
          messages: [
            _textMsg(
              id: 'msg1',
              pubkey: 'alice',
              content: 'Take this to #random',
              createdAt: 1000,
            ),
          ],
          users: {
            'alice': const UserProfile(pubkey: 'alice', displayName: 'Alice'),
          },
          channelsNotifier: channelsNotifier,
        ),
      );
      await tester.pumpAndSettle();

      channelsNotifier.setChannels([_testChannel]);
      await tester.tap(find.text('#random'));
      await tester.pump();

      expect(find.text('Channel could not be opened'), findsOneWidget);
    });

    testWidgets('tapping a channel link inside a thread opens that channel', (
      tester,
    ) async {
      final randomChannel = _channel(id: 'random-channel', name: 'random');
      final observer = _TestNavigatorObserver();

      await tester.pumpWidget(
        _buildTestable(
          messages: [
            _textMsg(
              id: 'msg1',
              pubkey: 'alice',
              content: 'Thread root #random',
              createdAt: 1000,
            ),
          ],
          users: {
            'alice': const UserProfile(pubkey: 'alice', displayName: 'Alice'),
          },
          channels: [_testChannel, randomChannel],
          navigatorObservers: [observer],
        ),
      );
      await tester.pumpAndSettle();

      final threadMessages = formatTimeline([
        _textMsg(
          id: 'msg1',
          pubkey: 'alice',
          content: 'Thread root #random',
          createdAt: 1000,
        ),
      ]);
      Navigator.of(tester.element(find.byType(ChannelDetailPage))).push(
        MaterialPageRoute<void>(
          builder: (_) => ThreadDetailPage(
            threadHead: threadMessages.single,
            allMessages: threadMessages,
            channelId: _channelId,
            currentPubkey: 'self',
            isMember: true,
            isArchived: false,
          ),
        ),
      );
      await tester.pumpAndSettle();
      final initialPushCount = observer.pushCount;

      await tester.tap(find.text('#random').last);
      await tester.pumpAndSettle();

      expect(observer.pushCount, initialPushCount + 1);
    });

    testWidgets('thread shows day dividers when replies cross days', (
      tester,
    ) async {
      final rootCreatedAt =
          DateTime(2025, 1, 1, 12).toUtc().millisecondsSinceEpoch ~/ 1000;
      final nextDayCreatedAt =
          DateTime(2025, 1, 2, 12).toUtc().millisecondsSinceEpoch ~/ 1000;
      final rootEvent = _textMsg(
        id: 'thread-root',
        pubkey: 'alice',
        content: 'Thread root',
        createdAt: rootCreatedAt,
      );
      final replies = [
        _textMsg(
          id: 'reply-same-day',
          pubkey: 'bob',
          content: 'Same day',
          createdAt: rootCreatedAt + 60,
          extraTags: const [
            ['e', 'thread-root', '', 'reply'],
          ],
        ),
        _textMsg(
          id: 'reply-next-day',
          pubkey: 'bob',
          content: 'Next day',
          createdAt: nextDayCreatedAt,
          extraTags: const [
            ['e', 'thread-root', '', 'reply'],
          ],
        ),
      ];

      await tester.pumpWidget(
        _buildTestable(
          messages: [rootEvent],
          threadReplies: {'thread-root': replies},
          users: {
            'alice': const UserProfile(pubkey: 'alice', displayName: 'Alice'),
            'bob': const UserProfile(pubkey: 'bob', displayName: 'Bob'),
          },
        ),
      );
      await tester.pumpAndSettle();

      final threadHead = formatTimeline([rootEvent]).single;
      Navigator.of(tester.element(find.byType(ChannelDetailPage))).push(
        MaterialPageRoute<void>(
          builder: (_) => ThreadDetailPage(
            threadHead: threadHead,
            allMessages: [threadHead],
            channelId: _channelId,
            currentPubkey: 'self',
            isMember: true,
            isArchived: false,
          ),
        ),
      );
      await tester.pumpAndSettle();

      expect(find.byType(DayDivider), findsNWidgets(2));
      expect(find.text(formatDayHeading(rootCreatedAt)), findsOneWidget);
      expect(find.text(formatDayHeading(nextDayCreatedAt)), findsOneWidget);
      final threadList = tester.widget<ScrollablePositionedList>(
        find.byKey(const ValueKey('thread-message-list')),
      );
      expect(threadList.padding!.bottom, 0);
      final newestThreadGroup = tester.widget<Padding>(
        find.byKey(const ValueKey('thread-message-group-reply-next-day')),
      );
      expect(newestThreadGroup.padding, const EdgeInsets.only(bottom: Grid.xs));
    });
  });
}

Channel _channel({required String id, required String name}) => Channel(
  id: id,
  name: name,
  channelType: 'stream',
  visibility: 'open',
  description: '$name discussion',
  createdBy: 'abc123',
  createdAt: DateTime(2025),
  memberCount: 3,
  isMember: true,
);

class _FakeMessagesNotifier extends ChannelMessagesNotifier {
  List<NostrEvent> _messages;
  bool _hasLoadedMessages;

  _FakeMessagesNotifier(this._messages, {bool hasLoadedMessages = true})
    : _hasLoadedMessages = hasLoadedMessages,
      super(_channelId);

  @override
  AsyncValue<List<NostrEvent>> build() => AsyncData(_messages);

  @override
  bool get hasLoadedMessages => _hasLoadedMessages;

  @override
  bool get reachedOldest => true;

  @override
  Future<bool> fetchOlder() async => false;

  void setMessages(List<NostrEvent> messages) {
    _messages = messages;
    _hasLoadedMessages = true;
    state = AsyncData(messages);
  }
}

class _ErrorMessagesNotifier extends ChannelMessagesNotifier {
  _ErrorMessagesNotifier() : super(_channelId);

  @override
  AsyncValue<List<NostrEvent>> build() =>
      AsyncError('Connection failed', StackTrace.current);
}

class _ReconnectingRelaySession extends RelaySessionNotifier {
  @override
  SessionState build() =>
      const SessionState(status: SessionStatus.reconnecting);

  @override
  Future<List<NostrEvent>> fetchHistory(
    NostrFilter filter, {
    Duration timeout = const Duration(seconds: 8),
  }) async => [];

  void connect() {
    state = const SessionState(status: SessionStatus.connected);
  }
}

class _FakeTypingNotifier extends ChannelTypingNotifier {
  final List<TypingEntry> _entries;
  _FakeTypingNotifier(this._entries) : super(_channelId);

  @override
  List<TypingEntry> build() => _entries;
}

class _SynchronousReadStateNotifier extends ReadStateNotifier {
  final ReadStateState _initialState;
  final Map<String, int> markedContexts = {};

  _SynchronousReadStateNotifier(this._initialState);

  @override
  ReadStateState build() => _initialState;

  @override
  void markContextRead(
    String contextId,
    int unixTimestamp, {
    bool clearForcedMessages = false,
  }) {
    markedContexts[contextId] = unixTimestamp;
    state = state.copyWithContext(contextId, unixTimestamp);
  }
}

class _FakeProfileNotifier extends ProfileNotifier {
  @override
  Future<UserProfile?> build() async =>
      const UserProfile(pubkey: 'self', displayName: 'Self');
}

class _FakeUserCacheNotifier extends UserCacheNotifier {
  final Map<String, UserProfile> _users;
  _FakeUserCacheNotifier(this._users);

  @override
  Map<String, UserProfile> build() => _users;

  @override
  UserProfile? get(String pubkey) => _users[pubkey.toLowerCase()];
}

class _FakeChannelsNotifier extends ChannelsNotifier {
  List<Channel> _channels;
  _FakeChannelsNotifier(this._channels);

  @override
  Future<List<Channel>> build() => SynchronousFuture(_channels);

  void setChannels(List<Channel> channels) {
    _channels = channels;
    state = AsyncData(channels);
  }
}

class _FakeChannelActions extends ChannelActions {
  final Future<void> Function(String channelId)? onJoinChannel;

  _FakeChannelActions(Ref ref, {this.onJoinChannel})
    : super(
        ref: ref,
        session: ref.read(relaySessionProvider.notifier),
        signedEventRelay: SignedEventRelay(
          session: ref.read(relaySessionProvider.notifier),
          nsec: null,
        ),
        currentPubkey: 'self',
      );

  @override
  Future<void> joinChannel(String channelId) async {
    await onJoinChannel?.call(channelId);
  }

  @override
  Future<void> leaveChannel(String channelId) async {
    return;
  }
}

class _TestNavigatorObserver extends NavigatorObserver {
  int pushCount = 0;

  @override
  void didPush(Route<dynamic> route, Route<dynamic>? previousRoute) {
    pushCount += 1;
    super.didPush(route, previousRoute);
  }
}
