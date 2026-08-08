import 'dart:async';
import 'dart:convert';

import 'package:flutter/foundation.dart';
import 'package:flutter/material.dart';
import 'package:flutter/rendering.dart' show ScrollDirection;
import 'package:flutter/services.dart';
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
import 'package:buzz/features/channels/emoji_picker.dart';
import 'package:buzz/features/channels/reaction_row.dart';
import 'package:buzz/features/channels/thread_detail_page.dart';
import 'package:buzz/features/channels/thread_replies_provider.dart';
import 'package:buzz/features/channels/timeline_message.dart';
import 'package:buzz/features/channels/channels_provider.dart';
import 'package:buzz/shared/read_state/read_state_provider.dart';
import 'package:buzz/features/channels/unread_badge/observed_unread_event.dart';
import 'package:buzz/features/channels/small_avatar.dart';
import 'package:buzz/features/profile/profile_provider.dart';
import 'package:buzz/features/profile/user_cache_provider.dart';
import 'package:buzz/features/profile/user_profile.dart';
import 'package:buzz/features/profile/user_profile_sheet.dart';
import 'package:buzz/shared/mentions/agent_identity_provider.dart';
import 'package:buzz/shared/relay/relay.dart';
import 'package:buzz/shared/theme/theme.dart';
import 'package:buzz/shared/widgets/frosted_app_bar.dart';
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

NostrEvent _huddleMsg({
  required String id,
  required int kind,
  String pubkey = 'alice',
  int createdAt = 1000,
}) => NostrEvent(
  id: id,
  pubkey: pubkey,
  createdAt: createdAt,
  kind: kind,
  tags: [
    ['h', _channelId],
  ],
  content: jsonEncode({
    'ephemeral_channel_id': '8d764100-fd8f-44cf-9c98-6d8fbd739b8c',
  }),
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
  _FakeUserCacheNotifier? userCacheNotifier,
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
  Map<String, Future<List<NostrEvent>>> pendingThreadReplies = const {},
  TextScaler textScaler = TextScaler.noScaling,
  bool disableAnimations = false,
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
      userCacheProvider.overrideWith(
        () => userCacheNotifier ?? _FakeUserCacheNotifier(users),
      ),
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
      channelBotPubkeysProvider(
        _channelId,
      ).overrideWith((ref) async => const <String>{}),
      if (createChannelActions != null)
        channelActionsProvider.overrideWith(createChannelActions),
      if (readStateNotifier != null)
        readStateProvider.overrideWith(() => readStateNotifier),
      for (final entry in threadReplies.entries)
        threadRepliesProvider(
          ThreadRepliesArgs(channelId: _channelId, rootId: entry.key),
        ).overrideWith((ref) async => entry.value),
      for (final entry in pendingThreadReplies.entries)
        threadRepliesProvider(
          ThreadRepliesArgs(channelId: _channelId, rootId: entry.key),
        ).overrideWith((ref) => entry.value),
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
        data: MediaQuery.of(context).copyWith(
          textScaler: textScaler,
          disableAnimations: disableAnimations,
        ),
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

Widget _buildNavigationTestable({
  required Channel channelA,
  required Channel channelB,
  required RelaySessionNotifier relaySession,
}) {
  return ProviderScope(
    overrides: [
      relaySessionProvider.overrideWith(() => relaySession),
      channelMessagesProvider(
        channelA.id,
      ).overrideWith(() => _FakeMessagesNotifier([], channelId: channelA.id)),
      channelMessagesProvider(
        channelB.id,
      ).overrideWith(() => _FakeMessagesNotifier([], channelId: channelB.id)),
      channelTypingProvider(
        channelA.id,
      ).overrideWith(() => _FakeTypingNotifier([], channelId: channelA.id)),
      channelTypingProvider(
        channelB.id,
      ).overrideWith(() => _FakeTypingNotifier([], channelId: channelB.id)),
      channelDetailsProvider(
        channelA.id,
      ).overrideWith((ref) async => ChannelDetails.fromChannel(channelA)),
      channelDetailsProvider(
        channelB.id,
      ).overrideWith((ref) async => ChannelDetails.fromChannel(channelB)),
      channelMembersProvider(
        channelA.id,
      ).overrideWith((ref) async => const <ChannelMember>[]),
      channelMembersProvider(
        channelB.id,
      ).overrideWith((ref) async => const <ChannelMember>[]),
      userCacheProvider.overrideWith(() => _FakeUserCacheNotifier({})),
      profileProvider.overrideWith(() => _FakeProfileNotifier()),
      channelsProvider.overrideWith(
        () => _FakeChannelsNotifier([channelA, channelB]),
      ),
      relayClientProvider.overrideWithValue(
        RelayClient(baseUrl: 'http://localhost:3000'),
      ),
      savedPrefsProvider.overrideWithValue(_testPrefs),
    ],
    child: MaterialApp(
      theme: AppTheme.light(),
      home: ChannelDetailPage(channel: channelA),
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
    testWidgets(
      'restores the previous channel replay priority after a nested pop',
      (tester) async {
        final channelA = _channel(id: 'channel-a', name: 'channel A');
        final channelB = _channel(id: 'channel-b', name: 'channel B');
        final socket = _RecordingRelaySocket();
        final relaySession = RelaySessionNotifier();
        relaySession.debugAttachSocketForTest(socket);

        final subscribeB = relaySession.subscribe(
          _filterForChannel(channelB.id),
          (_) {},
        );
        relaySession.debugHandleMessage(['EOSE', 'l-1']);
        await subscribeB;
        final subscribeA = relaySession.subscribe(
          _filterForChannel(channelA.id),
          (_) {},
        );
        relaySession.debugHandleMessage(['EOSE', 'l-2']);
        await subscribeA;

        await tester.pumpWidget(
          _buildNavigationTestable(
            channelA: channelA,
            channelB: channelB,
            relaySession: relaySession,
          ),
        );
        await tester.pumpAndSettle();

        final navigator = Navigator.of(
          tester.element(find.byType(ChannelDetailPage)),
        );
        navigator.push(
          MaterialPageRoute<void>(
            builder: (_) => ChannelDetailPage(channel: channelB),
          ),
        );
        await tester.pumpAndSettle();
        navigator.pop();
        await tester.pumpAndSettle();

        socket.messages.clear();
        await relaySession.debugReplayLiveSubscriptions();

        expect(_replayedChannelIds(socket), [channelA.id, channelB.id]);
      },
    );

    testWidgets(
      'keeps replacement channel replay priority after old route disposal',
      (tester) async {
        final channelA = _channel(id: 'channel-a', name: 'channel A');
        final channelB = _channel(id: 'channel-b', name: 'channel B');
        final socket = _RecordingRelaySocket();
        final relaySession = RelaySessionNotifier();
        relaySession.debugAttachSocketForTest(socket);

        final subscribeA = relaySession.subscribe(
          _filterForChannel(channelA.id),
          (_) {},
        );
        relaySession.debugHandleMessage(['EOSE', 'l-1']);
        await subscribeA;
        final subscribeB = relaySession.subscribe(
          _filterForChannel(channelB.id),
          (_) {},
        );
        relaySession.debugHandleMessage(['EOSE', 'l-2']);
        await subscribeB;

        await tester.pumpWidget(
          _buildNavigationTestable(
            channelA: channelA,
            channelB: channelB,
            relaySession: relaySession,
          ),
        );
        await tester.pumpAndSettle();

        Navigator.of(
          tester.element(find.byType(ChannelDetailPage)),
        ).pushReplacement(
          MaterialPageRoute<void>(
            builder: (_) => ChannelDetailPage(channel: channelB),
          ),
        );
        await tester.pumpAndSettle();

        socket.messages.clear();
        await relaySession.debugReplayLiveSubscriptions();

        expect(_replayedChannelIds(socket), [channelB.id, channelA.id]);
      },
    );

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

    testWidgets('clears the composer inset when membership is revoked', (
      tester,
    ) async {
      final channelsNotifier = _FakeChannelsNotifier([_testChannel]);
      await tester.pumpWidget(
        _buildTestable(
          messages: [
            _textMsg(
              id: 'msg1',
              pubkey: 'alice',
              content: 'Hello',
              createdAt: 1000,
            ),
          ],
          channelsNotifier: channelsNotifier,
        ),
      );
      await tester.pumpAndSettle();

      final messageListFinder = find.byKey(
        const ValueKey('channel-message-list'),
      );
      expect(
        tester
            .widget<ScrollablePositionedList>(messageListFinder)
            .padding!
            .bottom,
        greaterThan(0),
      );
      expect(
        find.byKey(const ValueKey('channel-composer-dock')),
        findsOneWidget,
      );

      channelsNotifier.setChannels([_testChannel.copyWith(isMember: false)]);
      await tester.pumpAndSettle();

      expect(find.byKey(const ValueKey('channel-composer-dock')), findsNothing);
      expect(
        tester
            .widget<ScrollablePositionedList>(messageListFinder)
            .padding!
            .bottom,
        0,
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

      await tester.tap(find.byTooltip('Channel actions'));
      await tester.pumpAndSettle();
      await tester.drag(
        find.byType(SingleChildScrollView).last,
        const Offset(0, -300),
      );
      await tester.pumpAndSettle();
      await tester.tap(find.text('Manage channel').last);
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

    testWidgets('detail-header manage leave closes the detail page', (
      tester,
    ) async {
      await tester.pumpWidget(
        _buildTestable(
          messages: const [],
          createChannelActions: (ref) => _FakeChannelActions(ref),
        ),
      );
      await tester.pumpAndSettle();

      await tester.tap(find.byTooltip('Channel actions'));
      await tester.pumpAndSettle();
      await tester.tap(find.text('Manage channel').last);
      await tester.pumpAndSettle();
      await tester.tap(find.text('Leave channel').last);
      await tester.pumpAndSettle();

      expect(find.byTooltip('Channel actions'), findsNothing);
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

      await tester.tap(find.byTooltip('Channel actions'));
      await tester.pumpAndSettle();
      await tester.drag(
        find.byType(SingleChildScrollView).last,
        const Offset(0, -300),
      );
      await tester.pumpAndSettle();
      await tester.tap(find.text('Manage channel').last);
      await tester.pumpAndSettle();

      final sheet = find.byType(BottomSheet).last;
      expect(find.byType(BottomSheet), findsNWidgets(2));
      expect(tester.getSize(sheet).height, lessThanOrEqualTo(720));

      final sheetTop = tester.getTopLeft(sheet).dy;
      await tester.dragFrom(
        Offset(tester.view.physicalSize.width / 2, sheetTop + 12),
        const Offset(0, 800),
      );
      await tester.pumpAndSettle();

      expect(find.text('Manage channel'), findsOneWidget);
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
      final composerDock = find.byKey(const ValueKey('channel-composer-dock'));
      final composerDockHeight = tester.getSize(composerDock).height;
      expect(messageList.padding!.bottom, composerDockHeight);
      expect(
        tester
            .getBottomLeft(find.byKey(const ValueKey('channel-message-list')))
            .dy,
        greaterThan(tester.getTopLeft(composerDock).dy),
      );
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

    testWidgets('long press opens the anchored reaction popover', (
      tester,
    ) async {
      final hapticCalls = <MethodCall>[];
      tester.binding.defaultBinaryMessenger.setMockMethodCallHandler(
        SystemChannels.platform,
        (call) async {
          if (call.method == 'HapticFeedback.vibrate') hapticCalls.add(call);
          return null;
        },
      );
      addTearDown(
        () => tester.binding.defaultBinaryMessenger.setMockMethodCallHandler(
          SystemChannels.platform,
          null,
        ),
      );

      await tester.pumpWidget(
        _buildTestable(
          messages: [
            _systemMsg(
              id: 'reaction-target',
              payload: {
                'type': 'member_joined',
                'actor': 'alice',
                'target': 'alice',
              },
            ),
          ],
          users: const {
            'alice': UserProfile(pubkey: 'alice', displayName: 'Alice'),
          },
        ),
      );
      await tester.pumpAndSettle();

      await tester.longPress(
        find.byKey(const ValueKey('system-message-row-reaction-target')),
      );
      await tester.pumpAndSettle();

      expect(find.byKey(const ValueKey('quick-reaction-more')), findsOneWidget);
      expect(
        find.byWidgetPredicate(
          (widget) =>
              widget.key is ValueKey<String> &&
              (widget.key! as ValueKey<String>).value.startsWith(
                'quick-reaction-',
              ),
        ),
        findsNWidgets(6),
      );
      expect(find.byType(BottomSheet), findsNothing);
      expect(find.text('Copy text'), findsNothing);
      expect(findRichText('joined the channel'), findsOneWidget);
      expect(hapticCalls, hasLength(1));
      expect(hapticCalls.single.arguments, 'HapticFeedbackType.mediumImpact');
    });

    testWidgets('reaction popover leaves existing reactions in the blur', (
      tester,
    ) async {
      await tester.pumpWidget(
        _buildTestable(
          messages: [
            _systemMsg(
              id: 'reacted-system-message',
              payload: {
                'type': 'member_joined',
                'actor': 'alice',
                'target': 'alice',
              },
            ),
            _reaction(
              id: 'existing-reaction',
              targetId: 'reacted-system-message',
            ),
          ],
          users: const {
            'alice': UserProfile(pubkey: 'alice', displayName: 'Alice'),
          },
        ),
      );
      await tester.pumpAndSettle();

      final messageCenter = tester.getCenter(
        findRichText('joined the channel'),
      );
      final reactionRect = tester.getRect(find.byType(ReactionRow));
      final reactionCenter = reactionRect.center;
      final reactionPillTop = Offset(
        reactionRect.center.dx,
        reactionRect.top + Grid.half + Grid.quarter,
      );
      await tester.longPress(
        find.byKey(const ValueKey('system-message-row-reacted-system-message')),
      );
      await tester.pumpAndSettle();

      final backgroundFinder = find.byKey(
        const ValueKey('reaction-popover-background'),
      );
      final background = tester.widget<ClipPath>(backgroundFinder);
      final backgroundOrigin = tester.getTopLeft(backgroundFinder);
      final blurPath = background.clipper!.getClip(
        tester.getSize(backgroundFinder),
      );

      expect(blurPath.contains(messageCenter - backgroundOrigin), isFalse);
      expect(blurPath.contains(reactionPillTop - backgroundOrigin), isTrue);
      expect(blurPath.contains(reactionCenter - backgroundOrigin), isTrue);
    });

    testWidgets('reaction popover grows right from a fixed left edge', (
      tester,
    ) async {
      await tester.pumpWidget(
        _buildTestable(
          messages: [
            _systemMsg(
              id: 'left-growth-target',
              payload: {
                'type': 'member_joined',
                'actor': 'alice',
                'target': 'alice',
              },
            ),
          ],
          users: const {
            'alice': UserProfile(pubkey: 'alice', displayName: 'Alice'),
          },
        ),
      );
      await tester.pumpAndSettle();

      await tester.longPress(
        find.byKey(const ValueKey('system-message-row-left-growth-target')),
      );
      await tester.pump();
      await tester.pump(const Duration(milliseconds: 100));
      final earlyRect = tester.getRect(
        find.byKey(const ValueKey('reaction-popover-tray')),
      );
      await tester.pump(const Duration(milliseconds: 80));
      final laterRect = tester.getRect(
        find.byKey(const ValueKey('reaction-popover-tray')),
      );

      expect(laterRect.left, moreOrLessEquals(earlyRect.left));
      expect(laterRect.width, greaterThan(earlyRect.width));
      await tester.pumpAndSettle();
    });

    testWidgets('long press survives a message rebuild during the hold', (
      tester,
    ) async {
      final userCache = _FakeUserCacheNotifier({
        'alice': const UserProfile(pubkey: 'alice', displayName: 'Alice'),
      });
      await tester.pumpWidget(
        _buildTestable(
          messages: [
            _systemMsg(
              id: 'rebuild-target',
              payload: {
                'type': 'member_joined',
                'actor': 'alice',
                'target': 'alice',
              },
            ),
          ],
          userCacheNotifier: userCache,
        ),
      );
      await tester.pumpAndSettle();

      final target = find.byKey(
        const ValueKey('system-message-row-rebuild-target'),
      );
      final gesture = await tester.startGesture(tester.getCenter(target));
      await tester.pump(const Duration(milliseconds: 250));
      userCache.replace(
        const UserProfile(pubkey: 'alice', displayName: 'Alice Updated'),
      );
      await tester.pump();
      await tester.pump(const Duration(milliseconds: 300));
      await gesture.up();
      await tester.pumpAndSettle();

      expect(
        find.byKey(const ValueKey('reaction-popover-tray')),
        findsOneWidget,
      );
    });

    testWidgets('long press works over nested rich message content', (
      tester,
    ) async {
      tester.view.physicalSize = const Size(400, 800);
      tester.view.devicePixelRatio = 1;
      addTearDown(tester.view.resetPhysicalSize);
      addTearDown(tester.view.resetDevicePixelRatio);

      const firstImage = 'https://example.com/media/first.png';
      const secondImage = 'https://example.com/media/second.png';
      await tester.pumpWidget(
        _buildTestable(
          messages: [
            _textMsg(
              id: 'rich-reaction-target',
              pubkey: 'alice',
              content:
                  'Gallery\n'
                  '![First]($firstImage)\n'
                  '![Second]($secondImage)',
              extraTags: const [
                ['imeta', 'url $firstImage', 'm image/png'],
                ['imeta', 'url $secondImage', 'm image/png'],
              ],
            ),
          ],
          users: const {
            'alice': UserProfile(pubkey: 'alice', displayName: 'Alice'),
          },
        ),
      );
      await tester.pumpAndSettle();

      await tester.longPress(
        find.byKey(const ValueKey('message-media-carousel')),
      );
      await tester.pumpAndSettle();

      expect(find.byKey(const ValueKey('reaction-popover-tray')), findsNothing);
      expect(find.byType(BottomSheet), findsOneWidget);
      expect(find.text('Copy text'), findsOneWidget);
    });

    testWidgets(
      'keeps image galleries body-aligned and flush with the trailing edge',
      (tester) async {
        tester.view.physicalSize = const Size(400, 800);
        tester.view.devicePixelRatio = 1;
        addTearDown(tester.view.resetPhysicalSize);
        addTearDown(tester.view.resetDevicePixelRatio);

        const firstImage = 'https://example.com/media/first.png';
        const secondImage = 'https://example.com/media/second.png';
        await tester.pumpWidget(
          _buildTestable(
            messages: [
              _textMsg(
                id: 'gallery',
                pubkey: 'alice',
                content:
                    'Gallery\n'
                    '![First]($firstImage)\n'
                    '![Second]($secondImage)',
                extraTags: const [
                  ['imeta', 'url $firstImage', 'm image/png'],
                  ['imeta', 'url $secondImage', 'm image/png'],
                ],
              ),
            ],
            users: const {
              'alice': UserProfile(pubkey: 'alice', displayName: 'Alice'),
            },
          ),
        );
        await tester.pumpAndSettle();

        final carousel = find.byKey(const ValueKey('message-media-carousel'));
        final imageCount = find.byKey(
          const ValueKey('message-media-carousel-count'),
        );
        final carouselRect = tester.getRect(carousel);
        final imageCountRect = tester.getRect(imageCount);

        expect(carouselRect.left, imageCountRect.left);
        expect(carouselRect.right, tester.view.physicalSize.width);
        expect(carouselRect.top - imageCountRect.bottom, Grid.half + 2);

        final messageMaterial = find
            .ancestor(
              of: find.byKey(const ValueKey('message-row-gallery')),
              matching: find.byType(Material),
            )
            .first;
        expect(
          tester.widget<Material>(messageMaterial).clipBehavior,
          Clip.none,
        );
      },
    );

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

    testWidgets('jumps to the oldest unread with compact inverse controls', (
      tester,
    ) async {
      final messages = [
        for (var i = 0; i < 40; i++)
          _textMsg(
            id: 'msg$i',
            pubkey: 'alice',
            content: 'Message $i',
            createdAt: 1000 + i,
          ),
      ];
      final channelsNotifier = _FakeChannelsNotifier(
        [_testChannel],
        observedUnread: {
          _channelId: [
            makeObservedUnreadEvent(
              id: 'msg21',
              createdAt: 1021,
              rootId: null,
              highPriority: false,
              channelType: 'stream',
              isThreadedReply: false,
            ),
          ],
        },
      );
      final readState = _SynchronousReadStateNotifier(
        const ReadStateState(
          isReady: true,
          pubkey: 'self',
          contexts: {_channelId: 1020},
          version: 0,
        ),
      );

      await tester.pumpWidget(
        _buildTestable(
          messages: messages,
          channelsNotifier: channelsNotifier,
          readStateNotifier: readState,
          users: const {
            'alice': UserProfile(pubkey: 'alice', displayName: 'Alice'),
          },
        ),
      );
      await tester.pumpAndSettle();

      final unreadButton = find.byKey(
        const ValueKey('channel-jump-to-oldest-unread'),
      );
      expect(unreadButton, findsOneWidget);
      expect(find.byTooltip('Jump to oldest unread message'), findsOneWidget);
      expect(find.byIcon(LucideIcons.chevronUp), findsOneWidget);
      expect(tester.getSize(unreadButton), const Size.square(48));
      final unreadRect = tester.getRect(unreadButton);
      expect(
        unreadRect.top,
        frostedAppBarHeight(tester.element(unreadButton)) + Grid.xs,
      );
      expect(find.text('Latest'), findsNothing);

      await tester.tap(unreadButton);
      await tester.pumpAndSettle();

      expect(findRichText('Message 21'), findsOneWidget);
      expect(unreadButton, findsNothing);
      expect(
        find.byKey(const ValueKey('channel-jump-to-latest')),
        findsOneWidget,
      );
      expect(find.text('Latest'), findsOneWidget);
      expect(find.byIcon(LucideIcons.arrowDown), findsOneWidget);
    });

    testWidgets('loads history through the oldest unread boundary', (
      tester,
    ) async {
      final newestPage = [
        for (var i = 50; i < 100; i++)
          _textMsg(
            id: 'msg$i',
            pubkey: 'alice',
            content: 'Message $i',
            createdAt: 1000 + i,
          ),
      ];
      final olderPage = [
        for (var i = 0; i < 50; i++)
          _textMsg(
            id: 'msg$i',
            pubkey: 'alice',
            content: 'Message $i',
            createdAt: 1000 + i,
          ),
      ];
      final messagesNotifier = _FakeMessagesNotifier(
        newestPage,
        olderPages: [olderPage],
      );
      final channelsNotifier = _FakeChannelsNotifier(
        [_testChannel],
        observedUnread: {
          _channelId: [
            makeObservedUnreadEvent(
              id: 'msg21',
              createdAt: 1021,
              rootId: null,
              highPriority: false,
              channelType: 'stream',
              isThreadedReply: false,
            ),
          ],
        },
      );
      final readState = _SynchronousReadStateNotifier(
        const ReadStateState(
          isReady: true,
          pubkey: 'self',
          contexts: {_channelId: 1020},
          version: 0,
        ),
      );

      await tester.pumpWidget(
        _buildTestable(
          messages: const [],
          messagesNotifier: messagesNotifier,
          channelsNotifier: channelsNotifier,
          readStateNotifier: readState,
          users: const {
            'alice': UserProfile(pubkey: 'alice', displayName: 'Alice'),
          },
        ),
      );
      await tester.pumpAndSettle();

      await tester.tap(
        find.byKey(const ValueKey('channel-jump-to-oldest-unread')),
      );
      await tester.pumpAndSettle();

      expect(findRichText('Message 21'), findsOneWidget);
      expect(findRichText('Message 50'), findsNothing);
      expect(messagesNotifier.fetchOlderCalls, 1);
    });

    testWidgets('does not load history for threaded-only unread events', (
      tester,
    ) async {
      final newestPage = [
        for (var i = 50; i < 100; i++)
          _textMsg(
            id: 'msg$i',
            pubkey: 'alice',
            content: 'Message $i',
            createdAt: 1000 + i,
          ),
      ];
      final olderPage = [
        for (var i = 0; i < 50; i++)
          _textMsg(
            id: 'msg$i',
            pubkey: 'alice',
            content: 'Message $i',
            createdAt: 1000 + i,
          ),
      ];
      final messagesNotifier = _FakeMessagesNotifier(
        newestPage,
        olderPages: [olderPage],
      );
      final channelsNotifier = _FakeChannelsNotifier(
        [_testChannel],
        observedUnread: {
          _channelId: [
            makeObservedUnreadEvent(
              id: 'thread-reply',
              createdAt: 1021,
              rootId: 'thread-root',
              highPriority: true,
              channelType: 'stream',
              isThreadedReply: true,
            ),
          ],
        },
      );
      final readState = _SynchronousReadStateNotifier(
        const ReadStateState(
          isReady: true,
          pubkey: 'self',
          contexts: {_channelId: 1020},
          version: 0,
        ),
      );

      await tester.pumpWidget(
        _buildTestable(
          messages: const [],
          messagesNotifier: messagesNotifier,
          channelsNotifier: channelsNotifier,
          readStateNotifier: readState,
        ),
      );
      await tester.pumpAndSettle();

      expect(messagesNotifier.fetchOlderCalls, 0);
      expect(
        find.byKey(const ValueKey('channel-jump-to-oldest-unread')),
        findsNothing,
      );
    });

    testWidgets('caps unread target history loading', (tester) async {
      final newestPage = [
        for (var i = 250; i < 300; i++)
          _textMsg(
            id: 'msg$i',
            pubkey: 'alice',
            content: 'Message $i',
            createdAt: 1000 + i,
          ),
      ];
      final olderPages = [
        for (var page = 4; page >= 0; page--)
          [
            for (var i = page * 50; i < (page + 1) * 50; i++)
              _textMsg(
                id: 'msg$i',
                pubkey: 'alice',
                content: 'Message $i',
                createdAt: 1000 + i,
              ),
          ],
      ];
      final messagesNotifier = _FakeMessagesNotifier(
        newestPage,
        olderPages: olderPages,
      );
      final channelsNotifier = _FakeChannelsNotifier(
        [_testChannel],
        observedUnread: {
          _channelId: [
            makeObservedUnreadEvent(
              id: 'missing-target',
              createdAt: 1001,
              rootId: null,
              highPriority: false,
              channelType: 'stream',
              isThreadedReply: false,
            ),
            makeObservedUnreadEvent(
              id: 'msg275',
              createdAt: 1275,
              rootId: null,
              highPriority: false,
              channelType: 'stream',
              isThreadedReply: false,
            ),
          ],
        },
      );
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
          messages: const [],
          messagesNotifier: messagesNotifier,
          channelsNotifier: channelsNotifier,
          readStateNotifier: readState,
        ),
      );
      await tester.pumpAndSettle();

      // History requests are deliberately scheduled after the current frame.
      // Advance the test clock until the capped chain has settled.
      for (
        var frame = 0;
        frame < 8 && messagesNotifier.fetchOlderCalls < 4;
        frame++
      ) {
        await tester.pump(const Duration(milliseconds: 1));
      }

      expect(messagesNotifier.fetchOlderCalls, 4);
      final unreadButton = find.byKey(
        const ValueKey('channel-jump-to-oldest-unread'),
      );
      expect(unreadButton, findsOneWidget);
      await tester.tap(unreadButton);
      await tester.pumpAndSettle();
      expect(findRichText('Message 275'), findsOneWidget);
    });

    testWidgets('stops loading the unread boundary after a failed page', (
      tester,
    ) async {
      final messages = [
        for (var i = 50; i < 100; i++)
          _textMsg(
            id: 'msg$i',
            pubkey: 'alice',
            content: 'Message $i',
            createdAt: 1000 + i,
          ),
      ];
      final messagesNotifier = _FakeMessagesNotifier(
        messages,
        failOlderFetch: true,
      );
      final readState = _SynchronousReadStateNotifier(
        const ReadStateState(
          isReady: true,
          pubkey: 'self',
          contexts: {_channelId: 1020},
          version: 0,
          forcedUnreadContexts: {_channelId: _channelId},
        ),
      );

      await tester.pumpWidget(
        _buildTestable(
          messages: const [],
          messagesNotifier: messagesNotifier,
          readStateNotifier: readState,
        ),
      );
      await tester.pumpAndSettle();

      expect(
        find.byKey(const ValueKey('channel-jump-to-oldest-unread')),
        findsNothing,
      );
      expect(find.bySemanticsLabel('Loading older messages'), findsNothing);
    });

    testWidgets('falls back when the oldest unread row is deleted', (
      tester,
    ) async {
      final messagesNotifier = _FakeMessagesNotifier([
        _textMsg(
          id: 'deleted-oldest',
          pubkey: 'alice',
          content: 'Deleted oldest',
          createdAt: 1021,
        ),
        _textMsg(
          id: 'reachable-unread',
          pubkey: 'alice',
          content: 'Reachable unread',
          createdAt: 1022,
        ),
        _deletion(
          id: 'delete-oldest',
          targetIds: ['deleted-oldest'],
          createdAt: 1023,
        ),
      ]);
      final channelsNotifier = _FakeChannelsNotifier(
        [_testChannel],
        observedUnread: {
          _channelId: [
            makeObservedUnreadEvent(
              id: 'deleted-oldest',
              createdAt: 1021,
              rootId: null,
              highPriority: false,
              channelType: 'stream',
              isThreadedReply: false,
            ),
            makeObservedUnreadEvent(
              id: 'reachable-unread',
              createdAt: 1022,
              rootId: null,
              highPriority: false,
              channelType: 'stream',
              isThreadedReply: false,
            ),
          ],
        },
      );
      final readState = _SynchronousReadStateNotifier(
        const ReadStateState(
          isReady: true,
          pubkey: 'self',
          contexts: {_channelId: 1020},
          version: 0,
        ),
      );

      await tester.pumpWidget(
        _buildTestable(
          messages: const [],
          messagesNotifier: messagesNotifier,
          channelsNotifier: channelsNotifier,
          readStateNotifier: readState,
          users: const {
            'alice': UserProfile(pubkey: 'alice', displayName: 'Alice'),
          },
        ),
      );
      await tester.pumpAndSettle();

      final unreadButton = find.byKey(
        const ValueKey('channel-jump-to-oldest-unread'),
      );
      expect(unreadButton, findsOneWidget);
      await tester.tap(unreadButton);
      await tester.pumpAndSettle();
      expect(findRichText('Reachable unread'), findsOneWidget);
    });

    testWidgets('pages past a loaded forced unread for an older target', (
      tester,
    ) async {
      final newestPage = [
        for (var i = 50; i < 100; i++)
          _textMsg(
            id: 'msg$i',
            pubkey: 'alice',
            content: 'Message $i',
            createdAt: 1000 + i,
          ),
      ];
      final olderPage = [
        for (var i = 0; i < 50; i++)
          _textMsg(
            id: 'msg$i',
            pubkey: 'alice',
            content: 'Message $i',
            createdAt: 1000 + i,
          ),
      ];
      final messagesNotifier = _FakeMessagesNotifier(
        newestPage,
        olderPages: [olderPage],
      );
      final channelsNotifier = _FakeChannelsNotifier(
        [_testChannel],
        observedUnread: {
          _channelId: [
            makeObservedUnreadEvent(
              id: 'msg21',
              createdAt: 1021,
              rootId: null,
              highPriority: false,
              channelType: 'stream',
              isThreadedReply: false,
            ),
          ],
        },
      );
      final readState = _SynchronousReadStateNotifier(
        const ReadStateState(
          isReady: true,
          pubkey: 'self',
          contexts: {_channelId: 1020},
          version: 0,
          forcedUnreadContexts: {'msg:msg75': _channelId},
        ),
      );

      await tester.pumpWidget(
        _buildTestable(
          messages: const [],
          messagesNotifier: messagesNotifier,
          channelsNotifier: channelsNotifier,
          readStateNotifier: readState,
          users: const {
            'alice': UserProfile(pubkey: 'alice', displayName: 'Alice'),
          },
        ),
      );
      await tester.pumpAndSettle();

      expect(messagesNotifier.fetchOlderCalls, 1);
      await tester.tap(
        find.byKey(const ValueKey('channel-jump-to-oldest-unread')),
      );
      await tester.pumpAndSettle();
      expect(findRichText('Message 21'), findsOneWidget);
      expect(findRichText('Message 75'), findsNothing);
    });

    testWidgets('targets the oldest message-level forced unread', (
      tester,
    ) async {
      final messages = [
        for (var i = 0; i < 40; i++)
          _textMsg(
            id: 'msg$i',
            pubkey: 'alice',
            content: 'Message $i',
            createdAt: 1000 + i,
          ),
      ];
      final readState = _SynchronousReadStateNotifier(
        const ReadStateState(
          isReady: true,
          pubkey: 'self',
          contexts: {_channelId: 2000},
          version: 0,
          forcedUnreadContexts: {
            'msg:msg20': _channelId,
            'msg:msg5': _channelId,
          },
        ),
      );

      await tester.pumpWidget(
        _buildTestable(
          messages: messages,
          readStateNotifier: readState,
          users: const {
            'alice': UserProfile(pubkey: 'alice', displayName: 'Alice'),
          },
        ),
      );
      await tester.pumpAndSettle();

      await tester.tap(
        find.byKey(const ValueKey('channel-jump-to-oldest-unread')),
      );
      await tester.pumpAndSettle();

      expect(findRichText('Message 5'), findsOneWidget);
      expect(findRichText('Message 20'), findsNothing);
    });

    testWidgets('ignores newer events absent from observed unread state', (
      tester,
    ) async {
      final messages = [
        _textMsg(
          id: 'read-message',
          pubkey: 'alice',
          content: 'Already read',
          createdAt: 1000,
        ),
        _textMsg(
          id: 'self-message',
          pubkey: 'self',
          content: 'My own newer message',
          createdAt: 1100,
        ),
        _systemMsg(
          id: 'system-message',
          payload: const {'type': 'channel_created'},
          createdAt: 1200,
        ),
      ];
      final readState = _SynchronousReadStateNotifier(
        const ReadStateState(
          isReady: true,
          pubkey: 'self',
          contexts: {_channelId: 1000},
          version: 0,
        ),
      );

      await tester.pumpWidget(
        _buildTestable(
          messages: messages,
          readStateNotifier: readState,
          users: const {
            'alice': UserProfile(pubkey: 'alice', displayName: 'Alice'),
          },
        ),
      );
      await tester.pumpAndSettle();

      expect(
        find.byKey(const ValueKey('channel-jump-to-oldest-unread')),
        findsNothing,
      );
    });

    testWidgets(
      'keeps the followed tail anchored through composer and keyboard resize',
      (tester) async {
        tester.view.physicalSize = const Size(400, 800);
        tester.view.devicePixelRatio = 1;
        addTearDown(tester.view.resetPhysicalSize);
        addTearDown(tester.view.resetDevicePixelRatio);
        addTearDown(tester.view.reset);

        final messages = [
          for (var i = 0; i < 20; i++)
            _textMsg(
              id: 'msg$i',
              pubkey: i.isEven ? 'alice' : 'bob',
              content: 'Message $i',
              createdAt: 1000 + i * 1000,
            ),
        ];

        await tester.pumpWidget(
          _buildTestable(
            messages: messages,
            users: const {
              'alice': UserProfile(pubkey: 'alice', displayName: 'Alice'),
              'bob': UserProfile(pubkey: 'bob', displayName: 'Bob'),
            },
          ),
        );
        await tester.pumpAndSettle();

        final latestMessage = find.byKey(
          const ValueKey('channel-message-group-msg19'),
        );
        final composerDock = find.byKey(
          const ValueKey('channel-composer-dock'),
        );
        final compactDockHeight = tester.getSize(composerDock).height;

        expect(latestMessage, findsOneWidget);
        expect(
          tester.getBottomLeft(latestMessage).dy,
          closeTo(tester.getTopLeft(composerDock).dy, 1),
        );

        await tester.tap(find.text('Message #general'));
        await tester.pumpAndSettle();

        expect(
          tester.getSize(composerDock).height,
          greaterThan(compactDockHeight),
        );
        expect(
          tester.getBottomLeft(latestMessage).dy,
          closeTo(tester.getTopLeft(composerDock).dy, 1),
        );
        expect(
          find.byKey(const ValueKey('channel-jump-to-latest')),
          findsNothing,
        );

        tester.view.viewInsets = const FakeViewPadding(bottom: 300);
        await tester.pumpAndSettle();

        expect(latestMessage, findsOneWidget);
        expect(
          tester.getBottomLeft(latestMessage).dy,
          closeTo(tester.getTopLeft(composerDock).dy, 1),
        );
        expect(
          find.byKey(const ValueKey('channel-jump-to-latest')),
          findsNothing,
        );
      },
    );

    testWidgets('keeps a short followed tail flush through composer resize', (
      tester,
    ) async {
      tester.view.physicalSize = const Size(400, 800);
      tester.view.devicePixelRatio = 1;
      addTearDown(tester.view.resetPhysicalSize);
      addTearDown(tester.view.resetDevicePixelRatio);
      addTearDown(tester.view.reset);

      final messages = [
        for (var i = 0; i < 3; i++)
          _textMsg(
            id: 'msg$i',
            pubkey: 'alice',
            content: 'Message $i',
            createdAt: 1000 + i,
          ),
      ];

      await tester.pumpWidget(
        _buildTestable(
          messages: messages,
          users: const {
            'alice': UserProfile(pubkey: 'alice', displayName: 'Alice'),
          },
        ),
      );
      await tester.pumpAndSettle();

      final latestMessage = find.byKey(
        const ValueKey('channel-message-group-msg2'),
      );
      final composerDock = find.byKey(const ValueKey('channel-composer-dock'));

      await tester.tap(find.text('Message #general'));
      await tester.pumpAndSettle();

      expect(
        tester.getBottomLeft(latestMessage).dy,
        closeTo(tester.getTopLeft(composerDock).dy, 1),
      );
      expect(
        find.byKey(const ValueKey('channel-jump-to-latest')),
        findsNothing,
      );
    });

    testWidgets(
      'does not realign a user-detached timeline on keyboard resize',
      (tester) async {
        tester.view.physicalSize = const Size(400, 600);
        tester.view.devicePixelRatio = 1;
        addTearDown(tester.view.resetPhysicalSize);
        addTearDown(tester.view.resetDevicePixelRatio);
        addTearDown(tester.view.reset);

        final messages = [
          for (var i = 0; i < 40; i++)
            _textMsg(
              id: 'msg$i',
              pubkey: 'alice',
              content: 'Message $i',
              createdAt: 1000 + i,
            ),
        ];

        await tester.pumpWidget(
          _buildTestable(
            messages: messages,
            users: const {
              'alice': UserProfile(pubkey: 'alice', displayName: 'Alice'),
            },
          ),
        );
        await tester.pumpAndSettle();

        final messageList = find.byKey(const ValueKey('channel-message-list'));
        await tester.drag(messageList, const Offset(0, 300));
        await tester.pumpAndSettle();

        expect(findRichText('Message 39'), findsNothing);
        expect(
          find.byKey(const ValueKey('channel-jump-to-latest')),
          findsOneWidget,
        );

        tester.view.viewInsets = const FakeViewPadding(bottom: 300);
        await tester.pumpAndSettle();

        expect(findRichText('Message 39'), findsNothing);
        expect(
          find.byKey(const ValueKey('channel-jump-to-latest')),
          findsOneWidget,
        );
        final positions = tester
            .widget<ScrollablePositionedList>(messageList)
            .itemPositionsNotifier!
            .itemPositions
            .value;
        expect(
          positions.any(
            (position) =>
                position.index == 0 && position.itemLeadingEdge.abs() < 0.01,
          ),
          isFalse,
        );
      },
    );

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
      final latestSurface = tester.widget<Container>(
        find.byKey(const ValueKey('channel-jump-to-latest-surface')),
      );
      final latestDecoration = latestSurface.decoration! as BoxDecoration;
      expect(latestDecoration.borderRadius, BorderRadius.circular(Radii.full));
      expect(
        latestDecoration.color,
        AppTheme.light().colorScheme.surface.withValues(alpha: 0.5),
      );
      expect(
        (latestDecoration.border! as Border).top.color,
        Colors.black.withValues(alpha: 0.04),
      );
      expect(
        find.descendant(
          of: find.byKey(const ValueKey('channel-jump-to-latest')),
          matching: find.byType(BackdropFilter),
        ),
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
      final latestMessage = find.byKey(
        const ValueKey('channel-message-group-newest'),
      );
      final composerDock = find.byKey(const ValueKey('channel-composer-dock'));
      expect(
        tester.getBottomLeft(latestMessage).dy,
        closeTo(tester.getTopLeft(composerDock).dy, 1),
      );
      expect(
        find.byKey(const ValueKey('channel-jump-to-latest')),
        findsNothing,
      );
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

        // Cache-extent mounting varies by platform, so assert the reversed
        // list's semantic boundary rather than whether item 0 is mounted.
        final positions = tester
            .widget<ScrollablePositionedList>(messageList)
            .itemPositionsNotifier!
            .itemPositions
            .value;
        expect(
          positions.any(
            (position) =>
                position.index == 0 && position.itemLeadingEdge.abs() < 0.01,
          ),
          isFalse,
        );
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
      'gives an initial deep link precedence over unread navigation',
      (tester) async {
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
        final channelsNotifier = _FakeChannelsNotifier(
          [_testChannel],
          observedUnread: {
            _channelId: [
              makeObservedUnreadEvent(
                id: 'msg5',
                createdAt: 1005,
                rootId: null,
                highPriority: false,
                channelType: 'stream',
                isThreadedReply: false,
              ),
            ],
          },
        );
        final readState = _SynchronousReadStateNotifier(
          const ReadStateState(
            isReady: true,
            pubkey: 'self',
            contexts: {_channelId: 1004},
            version: 0,
          ),
        );

        await tester.pumpWidget(
          _buildTestable(
            messages: const [],
            messagesNotifier: messagesNotifier,
            channelsNotifier: channelsNotifier,
            readStateNotifier: readState,
            initialMessageId: 'msg20',
            users: const {
              'alice': UserProfile(pubkey: 'alice', displayName: 'Alice'),
            },
          ),
        );
        await tester.pumpAndSettle();

        expect(findRichText('Message 20'), findsOneWidget);
        expect(findRichText('Message 5'), findsNothing);
        expect(
          find.byKey(const ValueKey('channel-jump-to-oldest-unread')),
          findsNothing,
        );
        expect(messagesNotifier.fetchOlderCalls, 0);
      },
    );

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
        //
        // Each drag must clear `kDragSlopDefault`; below it `tester.drag`
        // sends a single sub-slop move, which a message bubble now claims as
        // a tap and pushes the thread page over this list.
        for (final dy in const [30.0, 30.0, 30.0]) {
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

    testWidgets('renders a huddle event like a regular message row', (
      tester,
    ) async {
      await tester.pumpWidget(
        _buildTestable(
          messages: [_huddleMsg(id: 'huddle-1', kind: EventKind.huddleStarted)],
          users: {
            'alice': const UserProfile(pubkey: 'alice', displayName: 'Alice'),
          },
        ),
      );
      await tester.pumpAndSettle();

      expect(find.text('Alice'), findsOneWidget);
      expect(findRichText('started a huddle'), findsOneWidget);
      expect(
        tester.getSize(find.byType(CircleAvatar)),
        const Size.square(messageAvatarSize),
      );
      expect(
        find.byKey(const ValueKey('system-message-timestamp-alice')),
        findsOneWidget,
      );
    });

    testWidgets(
      'keeps membership and huddle rows evenly spaced with authored messages',
      (tester) async {
        await tester.pumpWidget(
          _buildTestable(
            messages: [
              _textMsg(
                id: 'message-alice',
                pubkey: 'alice',
                content: 'First message',
                createdAt: 1000,
              ),
              _textMsg(
                id: 'message-bob',
                pubkey: 'bob',
                content: 'Second message',
                createdAt: 1010,
              ),
              _systemMsg(
                id: 'membership-carol',
                payload: {
                  'type': 'member_joined',
                  'actor': 'alice',
                  'target': 'carol',
                },
                createdAt: 1020,
              ),
              _huddleMsg(
                id: 'huddle-dave',
                kind: EventKind.huddleStarted,
                pubkey: 'dave',
                createdAt: 1030,
              ),
              _textMsg(
                id: 'message-erin',
                pubkey: 'erin',
                content: 'Third message',
                createdAt: 1040,
              ),
            ],
            users: {
              for (final name in ['alice', 'bob', 'carol', 'dave', 'erin'])
                name: UserProfile(pubkey: name, displayName: name),
            },
          ),
        );
        await tester.pumpAndSettle();

        Rect avatarRect(String rowKey) => tester.getRect(
          find
              .descendant(
                of: find.byKey(ValueKey(rowKey)),
                matching: find.byType(CircleAvatar),
              )
              .first,
        );

        final avatars = [
          avatarRect('message-row-message-alice'),
          avatarRect('message-row-message-bob'),
          avatarRect('system-message-row-membership-carol'),
          avatarRect('system-message-row-huddle-dave'),
          avatarRect('message-row-message-erin'),
        ];
        final authoredMessageGap = avatars[1].top - avatars[0].bottom;

        for (var index = 2; index < avatars.length; index++) {
          expect(
            avatars[index].top - avatars[index - 1].bottom,
            closeTo(authoredMessageGap, 1),
            reason: 'row $index should use the authored-message gap',
          );
        }
      },
    );

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

    testWidgets('opens a profile sheet from a membership system avatar', (
      tester,
    ) async {
      await tester.pumpWidget(
        _buildTestable(
          messages: [
            _systemMsg(
              id: 'sys-membership-avatar',
              payload: {
                'type': 'member_joined',
                'actor': 'alice',
                'target': 'bob',
              },
            ),
          ],
          users: {
            'alice': const UserProfile(pubkey: 'alice', displayName: 'Alice'),
            'bob': const UserProfile(pubkey: 'bob', displayName: 'Bob'),
          },
        ),
      );
      await tester.pumpAndSettle();

      await tester.tap(find.byType(CircleAvatar));
      await tester.pumpAndSettle();

      expect(find.text('Copy public key'), findsOneWidget);
      expect(find.text('alice'), findsNothing);
      expect(find.byType(UserProfileSheet), findsOneWidget);

      TestDefaultBinaryMessengerBinding.instance.defaultBinaryMessenger
          .setMockMethodCallHandler(SystemChannels.platform, (_) async => null);
      addTearDown(
        () => TestDefaultBinaryMessengerBinding.instance.defaultBinaryMessenger
            .setMockMethodCallHandler(SystemChannels.platform, null),
      );
      await tester.ensureVisible(find.text('Copy public key'));
      await tester.pumpAndSettle();
      final copyAction = find
          .ancestor(
            of: find.text('Copy public key'),
            matching: find.byType(GestureDetector),
          )
          .last;
      tester.widget<GestureDetector>(copyAction).onTap!();
      await tester.pump();
      await tester.pump();
      expect(find.text('Public key copied'), findsOneWidget);

      await tester.tap(find.byTooltip('Close sheet'));
      await tester.pumpAndSettle();
      await tester.pump(const Duration(seconds: 2));

      expect(tester.takeException(), isNull);
    });

    testWidgets('opens a profile sheet from a huddle system avatar', (
      tester,
    ) async {
      await tester.pumpWidget(
        _buildTestable(
          messages: [
            _huddleMsg(
              id: 'sys-huddle-avatar',
              kind: EventKind.huddleStarted,
              pubkey: 'alice',
            ),
          ],
          users: {
            'alice': const UserProfile(pubkey: 'alice', displayName: 'Alice'),
          },
        ),
      );
      await tester.pumpAndSettle();

      await tester.tap(find.byType(CircleAvatar));
      await tester.pumpAndSettle();

      expect(find.text('Copy public key'), findsOneWidget);
      expect(find.byType(UserProfileSheet), findsOneWidget);
    });

    testWidgets('opens a profile sheet from a generic system avatar', (
      tester,
    ) async {
      await tester.pumpWidget(
        _buildTestable(
          messages: [
            _systemMsg(
              id: 'sys-removed-avatar',
              payload: {
                'type': 'member_removed',
                'actor': 'alice',
                'target': 'bob',
              },
            ),
          ],
          users: {
            'alice': const UserProfile(pubkey: 'alice', displayName: 'Alice'),
            'bob': const UserProfile(pubkey: 'bob', displayName: 'Bob'),
          },
        ),
      );
      await tester.pumpAndSettle();

      await tester.tap(find.byType(CircleAvatar).first);
      await tester.pumpAndSettle();

      expect(find.text('Copy public key'), findsOneWidget);
      expect(find.byType(UserProfileSheet), findsOneWidget);
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
      final addedAction = findRichText('added by Alice');
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
        effectiveFontSizeForText(addedText.text, 'added by Alice'),
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
        findRichText('added by Alice, along with Carol, Dave, Erin, and '),
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

    testWidgets('a reacted message offers the + picker in the timeline', (
      tester,
    ) async {
      // Desktop puts the picker trigger beside existing reactions on every row,
      // so reacting doesn't require discovering the long-press sheet.
      await tester.pumpWidget(
        _buildTestable(
          messages: [
            _textMsg(id: 'msg1', pubkey: 'alice', content: 'ship it'),
            _reaction(id: 'reaction-1', targetId: 'msg1'),
          ],
          users: {
            'alice': const UserProfile(pubkey: 'alice', displayName: 'Alice'),
          },
        ),
      );
      await tester.pumpAndSettle();

      expect(find.byKey(const ValueKey('add-reaction-pill')), findsOneWidget);

      await tester.tap(find.byKey(const ValueKey('add-reaction-pill')));
      // Not pumpAndSettle: with no dataset asset in a widget test the sheet
      // shows its loading spinner, which animates forever.
      await tester.pump();
      await tester.pump(const Duration(milliseconds: 400));
      expect(find.byType(EmojiPickerSheet), findsOneWidget);
    });

    testWidgets('an unreacted message keeps the timeline free of chrome', (
      tester,
    ) async {
      await tester.pumpWidget(
        _buildTestable(
          messages: [_textMsg(id: 'msg1', pubkey: 'alice', content: 'ship it')],
          users: {
            'alice': const UserProfile(pubkey: 'alice', displayName: 'Alice'),
          },
        ),
      );
      await tester.pumpAndSettle();

      // No reactions, no row — the + only trails reactions that already exist.
      expect(find.byKey(const ValueKey('add-reaction-pill')), findsNothing);
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
      await tester.pump();
      await tester.pump(const Duration(milliseconds: 200));

      expect(find.text('Alice is typing…'), findsOneWidget);

      final indicator = tester.widget<Container>(
        find.byKey(const ValueKey('channel-typing-indicator')),
      );
      final decoration = indicator.decoration! as BoxDecoration;
      expect(
        indicator.padding,
        const EdgeInsets.symmetric(horizontal: Grid.xxs, vertical: Grid.xxs),
      );
      expect(
        decoration.color,
        AppTheme.light().colorScheme.surfaceContainerHighest,
      );
      expect(decoration.border, isA<Border>());
      expect(
        tester.widget<Text>(find.text('Alice is typing…')).style?.color,
        AppTheme.light().colorScheme.onSurfaceVariant,
      );
      expect(
        tester.widget<Text>(find.text('Alice is typing…')).style?.fontStyle,
        isNot(FontStyle.italic),
      );
      expect(
        find.byKey(const ValueKey('channel-typing-shimmer')),
        findsOneWidget,
      );
      expect(tester.widget<SmallAvatar>(find.byType(SmallAvatar)).size, 24);
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
      await tester.pump();
      await tester.pump(const Duration(milliseconds: 200));

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
      await tester.pump();
      await tester.pump(const Duration(milliseconds: 200));

      expect(find.text('Alice and 2 others are typing…'), findsOneWidget);
    });

    testWidgets('keeps typing text static when motion is reduced', (
      tester,
    ) async {
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
          disableAnimations: true,
        ),
      );
      await tester.pump();
      await tester.pump();

      expect(find.text('Alice is typing…'), findsOneWidget);
      expect(
        find.byKey(const ValueKey('channel-typing-shimmer')),
        findsNothing,
      );
    });
  });

  group('Compose bar', () {
    testWidgets('expands from the channel hint into the composer controls', (
      tester,
    ) async {
      await tester.pumpWidget(_buildTestable(messages: []));
      await tester.pumpAndSettle();

      expect(find.byType(TextField), findsNothing);
      expect(find.byIcon(LucideIcons.arrowUp).hitTestable(), findsOneWidget);

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
      // The list runs top-down (head first), so tail spacing lives on the list
      // and reply groups carry none.
      final threadList = tester.widget<ScrollablePositionedList>(
        find.byKey(const ValueKey('thread-message-list')),
      );
      expect(threadList.reverse, isFalse);
      final threadComposerDockHeight = tester
          .getSize(find.byKey(const ValueKey('thread-composer-dock')))
          .height;
      expect(threadList.padding!.bottom, Grid.xs + threadComposerDockHeight);
      final newestThreadGroup = tester.widget<Padding>(
        find.byKey(const ValueKey('thread-message-group-reply-next-day')),
      );
      expect(newestThreadGroup.padding, EdgeInsets.zero);

      // The head sits above its replies rather than jammed against the
      // composer, matching desktop's thread panel.
      final headY = tester
          .getTopLeft(
            find.byKey(const ValueKey('thread-message-group-thread-root')),
          )
          .dy;
      final oldestReplyY = tester
          .getTopLeft(
            find.byKey(const ValueKey('thread-message-group-reply-same-day')),
          )
          .dy;
      final newestReplyY = tester
          .getTopLeft(
            find.byKey(const ValueKey('thread-message-group-reply-next-day')),
          )
          .dy;
      expect(headY, lessThan(oldestReplyY));
      expect(oldestReplyY, lessThan(newestReplyY));
    });

    testWidgets('thread keeps its tail above a growing composer dock', (
      tester,
    ) async {
      tester.view.physicalSize = const Size(400, 800);
      tester.view.devicePixelRatio = 1;
      addTearDown(tester.view.resetPhysicalSize);
      addTearDown(tester.view.resetDevicePixelRatio);

      final rootEvent = _textMsg(
        id: 'thread-root',
        pubkey: 'alice',
        content: 'Thread root',
        createdAt: 1000,
      );
      final replies = [
        for (var i = 0; i < 20; i++)
          _textMsg(
            id: 'reply-$i',
            pubkey: 'bob',
            content: 'Reply $i',
            createdAt: 1100 + i,
            extraTags: const [
              ['e', 'thread-root', '', 'reply'],
            ],
          ),
      ];

      await tester.pumpWidget(
        _buildTestable(
          messages: [rootEvent],
          threadReplies: {'thread-root': replies},
          users: const {
            'alice': UserProfile(pubkey: 'alice', displayName: 'Alice'),
            'bob': UserProfile(pubkey: 'bob', displayName: 'Bob'),
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
            initialMessageId: 'reply-19',
          ),
        ),
      );
      await tester.pumpAndSettle();

      final dock = find.byKey(const ValueKey('thread-composer-dock'));
      final latestReply = find.byKey(
        const ValueKey('thread-message-group-reply-19'),
      );
      final composerSurface = find.byKey(const ValueKey('composer-surface'));
      final compactDockHeight = tester.getSize(dock).height;
      expect(latestReply, findsOneWidget);
      expect(
        tester.getBottomLeft(latestReply).dy,
        lessThanOrEqualTo(tester.getTopLeft(composerSurface).dy),
      );

      await tester.tap(find.text('Reply in thread…').hitTestable());
      await tester.pumpAndSettle();

      expect(tester.getSize(dock).height, greaterThan(compactDockHeight));
      expect(latestReply, findsOneWidget);
      expect(
        tester.getBottomLeft(latestReply).dy,
        lessThanOrEqualTo(tester.getTopLeft(composerSurface).dy),
      );

      // The dock size change above is separate from the later Scaffold
      // viewport resize caused by the keyboard. Keep following the tail after
      // that metrics change too.
      tester.view.viewInsets = const FakeViewPadding(bottom: 300);
      addTearDown(tester.view.reset);
      await tester.pumpAndSettle();

      expect(
        tester.getBottomLeft(latestReply).dy,
        lessThanOrEqualTo(tester.getTopLeft(composerSurface).dy),
      );
    });

    testWidgets(
      'initial thread hydration keeps the head visible instead of following the tail',
      (tester) async {
        final rootEvent = _textMsg(
          id: 'thread-root',
          pubkey: 'alice',
          content: 'Thread root',
          createdAt: 1000,
        );
        final replies = [
          for (var i = 0; i < 30; i++)
            _textMsg(
              id: 'reply-$i',
              pubkey: 'bob',
              content: 'Reply $i',
              createdAt: 1100 + i,
              extraTags: const [
                ['e', 'thread-root', '', 'reply'],
              ],
            ),
        ];
        final completer = Completer<List<NostrEvent>>();

        await tester.pumpWidget(
          _buildTestable(
            messages: [rootEvent],
            pendingThreadReplies: {'thread-root': completer.future},
            users: const {
              'alice': UserProfile(pubkey: 'alice', displayName: 'Alice'),
              'bob': UserProfile(pubkey: 'bob', displayName: 'Bob'),
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
        expect(
          find.byKey(const ValueKey('thread-message-group-thread-root')),
          findsOneWidget,
        );

        completer.complete(replies);
        await tester.pumpAndSettle();

        expect(
          find.byKey(const ValueKey('thread-message-group-thread-root')),
          findsOneWidget,
        );
        expect(
          find.byKey(const ValueKey('thread-message-group-reply-29')),
          findsNothing,
        );
      },
    );

    testWidgets(
      'deep-linking an older reply does not resume tail following on keyboard resize',
      (tester) async {
        tester.view.physicalSize = const Size(400, 800);
        tester.view.devicePixelRatio = 1;
        addTearDown(tester.view.resetPhysicalSize);
        addTearDown(tester.view.resetDevicePixelRatio);
        addTearDown(tester.view.reset);

        final rootEvent = _textMsg(
          id: 'thread-root',
          pubkey: 'alice',
          content: 'Thread root',
          createdAt: 1000,
        );
        final replies = [
          for (var i = 0; i < 30; i++)
            _textMsg(
              id: 'reply-$i',
              pubkey: 'bob',
              content: 'Reply $i',
              createdAt: 1100 + i,
              extraTags: const [
                ['e', 'thread-root', '', 'reply'],
              ],
            ),
        ];
        final completer = Completer<List<NostrEvent>>();

        await tester.pumpWidget(
          _buildTestable(
            messages: [rootEvent],
            pendingThreadReplies: {'thread-root': completer.future},
            users: const {
              'alice': UserProfile(pubkey: 'alice', displayName: 'Alice'),
              'bob': UserProfile(pubkey: 'bob', displayName: 'Bob'),
            },
          ),
        );
        await tester.pumpAndSettle();

        final threadHead = formatTimeline([rootEvent]).single;
        final provisionalTarget = formatTimeline([replies[5]]).single;
        Navigator.of(tester.element(find.byType(ChannelDetailPage))).push(
          MaterialPageRoute<void>(
            builder: (_) => ThreadDetailPage(
              threadHead: threadHead,
              allMessages: [threadHead, provisionalTarget],
              channelId: _channelId,
              currentPubkey: 'self',
              isMember: true,
              isArchived: false,
              initialMessageId: 'reply-5',
            ),
          ),
        );
        await tester.pumpAndSettle();

        completer.complete(replies);
        await tester.pumpAndSettle();

        final target = find.byKey(
          const ValueKey('thread-message-group-reply-5'),
        );
        expect(target, findsOneWidget);

        await tester.tap(find.text('Reply in thread…').hitTestable());
        await tester.pumpAndSettle();
        tester.view.viewInsets = const FakeViewPadding(bottom: 300);
        await tester.pumpAndSettle();

        expect(target, findsOneWidget);
      },
    );

    testWidgets('a reaction landing while the thread is open shows up there', (
      tester,
    ) async {
      // The thread's own relay query is one-shot and asks only for content
      // kinds, so it can never carry a reaction that arrives afterwards. Until
      // the live channel events were folded in, the pill (and its burst) only
      // appeared after leaving the thread and coming back, which refetched.
      final rootEvent = _textMsg(
        id: 'thread-root',
        pubkey: 'alice',
        content: 'Thread root',
        createdAt: 1000,
      );
      final reply = _textMsg(
        id: 'reply-1',
        pubkey: 'bob',
        content: 'A reply',
        createdAt: 1100,
        extraTags: const [
          ['e', 'thread-root', '', 'reply'],
        ],
      );
      final messagesNotifier = _FakeMessagesNotifier([rootEvent]);

      await tester.pumpWidget(
        _buildTestable(
          messages: [rootEvent],
          messagesNotifier: messagesNotifier,
          threadReplies: {
            'thread-root': [reply],
          },
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
      expect(find.byKey(const ValueKey('reaction-pill-👍')), findsNothing);

      // The reaction arrives over the channel socket, as it does on device.
      messagesNotifier.setMessages([
        rootEvent,
        _reaction(id: 'reaction-1', targetId: 'reply-1'),
      ]);
      await tester.pumpAndSettle();

      // Still on the thread route — no pop needed for the pill to appear.
      expect(find.byType(ThreadDetailPage), findsOneWidget);
      expect(find.byKey(const ValueKey('reaction-pill-👍')), findsOneWidget);
    });

    testWidgets('a live deletion does not restore the routed thread head', (
      tester,
    ) async {
      final rootEvent = _textMsg(
        id: 'thread-root',
        pubkey: 'alice',
        content: 'Thread root',
        createdAt: 1000,
      );
      final reply = _textMsg(
        id: 'reply-1',
        pubkey: 'bob',
        content: 'A reply',
        createdAt: 1100,
        extraTags: const [
          ['e', 'thread-root', '', 'reply'],
        ],
      );
      final messagesNotifier = _FakeMessagesNotifier([rootEvent]);

      await tester.pumpWidget(
        _buildTestable(
          messages: [rootEvent],
          messagesNotifier: messagesNotifier,
          threadReplies: {
            'thread-root': [reply],
          },
          users: const {
            'alice': UserProfile(pubkey: 'alice', displayName: 'Alice'),
            'bob': UserProfile(pubkey: 'bob', displayName: 'Bob'),
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
      expect(find.text('Thread root'), findsOneWidget);

      messagesNotifier.setMessages([
        rootEvent,
        _deletion(id: 'delete-root', targetIds: ['thread-root']),
      ]);
      await tester.pumpAndSettle();

      expect(find.text('Thread root'), findsNothing);
      expect(
        find.byKey(const ValueKey('thread-message-deleted')),
        findsOneWidget,
      );
      expect(find.text('This message was deleted'), findsOneWidget);
    });

    testWidgets('thread replies earn the + only once they carry a reaction', (
      tester,
    ) async {
      final rootEvent = _textMsg(
        id: 'thread-root',
        pubkey: 'alice',
        content: 'Thread root',
        createdAt: 1000,
      );
      final bareReply = _textMsg(
        id: 'reply-bare',
        pubkey: 'bob',
        content: 'No reactions here',
        createdAt: 1100,
        extraTags: const [
          ['e', 'thread-root', '', 'reply'],
        ],
      );
      final reactedReply = _textMsg(
        id: 'reply-reacted',
        pubkey: 'bob',
        content: 'This one has a reaction',
        createdAt: 1200,
        extraTags: const [
          ['e', 'thread-root', '', 'reply'],
        ],
      );

      await tester.pumpWidget(
        _buildTestable(
          messages: [
            rootEvent,
            _reaction(id: 'reaction-1', targetId: 'reply-reacted'),
          ],
          threadReplies: {
            'thread-root': [bareReply, reactedReply],
          },
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

      // The head keeps a standing +; the bare reply gets none, so a thread
      // reads as quietly as the channel does. Two + pills, not three: the head
      // and the reacted reply.
      expect(find.byKey(const ValueKey('add-reaction-pill')), findsNWidgets(2));
      final headRow = find.descendant(
        of: find.byKey(const ValueKey('thread-message-group-thread-root')),
        matching: find.byKey(const ValueKey('add-reaction-pill')),
      );
      expect(headRow, findsOneWidget);
      expect(
        find.descendant(
          of: find.byKey(const ValueKey('thread-message-group-reply-bare')),
          matching: find.byKey(const ValueKey('add-reaction-pill')),
        ),
        findsNothing,
      );
      expect(
        find.descendant(
          of: find.byKey(const ValueKey('thread-message-group-reply-reacted')),
          matching: find.byKey(const ValueKey('add-reaction-pill')),
        ),
        findsOneWidget,
      );
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
  final List<List<NostrEvent>> _olderPages;
  final bool failOlderFetch;
  int fetchOlderCalls = 0;

  _FakeMessagesNotifier(
    this._messages, {
    String channelId = _channelId,
    bool hasLoadedMessages = true,
    List<List<NostrEvent>> olderPages = const [],
    this.failOlderFetch = false,
  }) : _hasLoadedMessages = hasLoadedMessages,
       _olderPages = [...olderPages],
       super(channelId);

  @override
  AsyncValue<List<NostrEvent>> build() => AsyncData(_messages);

  @override
  bool get hasLoadedMessages => _hasLoadedMessages;

  @override
  bool get reachedOldest => _olderPages.isEmpty && !failOlderFetch;

  @override
  Future<bool> fetchOlder() async {
    fetchOlderCalls += 1;
    if (failOlderFetch || _olderPages.isEmpty) return false;
    _messages = [..._olderPages.removeAt(0), ..._messages]
      ..sort((a, b) => a.createdAt.compareTo(b.createdAt));
    state = AsyncData(_messages);
    return true;
  }

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
  _FakeTypingNotifier(this._entries, {String channelId = _channelId})
    : super(channelId);

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

  void replace(UserProfile profile) {
    state = {...state, profile.pubkey.toLowerCase(): profile};
  }
}

class _FakeChannelsNotifier extends ChannelsNotifier {
  List<Channel> _channels;
  final Map<String, Map<String, ObservedUnreadEvent>> _observedUnread;

  _FakeChannelsNotifier(
    this._channels, {
    Map<String, List<ObservedUnreadEvent>> observedUnread = const {},
  }) : _observedUnread = {
         for (final entry in observedUnread.entries)
           entry.key: {for (final event in entry.value) event.id: event},
       };

  @override
  Map<String, Map<String, ObservedUnreadEvent>>
  get observedUnreadEventsByChannel => _observedUnread;

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

class _RecordingRelaySocket extends RelaySocket {
  _RecordingRelaySocket()
    : super(
        wsUrl: 'wss://relay.example',
        nsec: null,
        onMessage: (_) {},
        onConnected: () {},
        onDisconnected: (_) {},
      );

  final List<List<dynamic>> messages = [];

  @override
  void send(List<dynamic> payload) => messages.add(payload);

  @override
  void dispose() {}
}

NostrFilter _filterForChannel(String channelId) => NostrFilter(
  kinds: EventKind.channelEventKinds,
  tags: {
    '#h': [channelId],
  },
  limit: 0,
);

List<String> _replayedChannelIds(_RecordingRelaySocket socket) => socket
    .messages
    .where((message) => message.first == 'REQ')
    .map(
      (message) =>
          ((message[2] as Map<String, dynamic>)['#h'] as List).single as String,
    )
    .toList();

class _TestNavigatorObserver extends NavigatorObserver {
  int pushCount = 0;

  @override
  void didPush(Route<dynamic> route, Route<dynamic>? previousRoute) {
    pushCount += 1;
    super.didPush(route, previousRoute);
  }
}
