import 'dart:async';
import 'dart:collection';
import 'dart:convert';

import 'package:flutter/foundation.dart';
import 'package:flutter/material.dart';
import 'package:flutter/rendering.dart'
    show RenderParagraph, ScrollDirection, SemanticsAction;
import 'package:flutter/services.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';
import 'package:http/http.dart' as http;
import 'package:http/testing.dart' as http_testing;
import 'package:lucide_icons_flutter/lucide_icons.dart';
import 'package:nostr/nostr.dart' as nostr;
import 'package:pointycastle/digests/sha256.dart';
import 'package:scrollable_positioned_list/scrollable_positioned_list.dart';
import 'package:buzz/features/channels/channel.dart';
import 'package:buzz/features/channels/channel_detail_page.dart';
import 'package:buzz/features/channels/channel_management_provider.dart';
import 'package:buzz/features/channels/channel_messages_provider.dart';
import 'package:buzz/features/channels/channel_mutes/channel_mutes_provider.dart';
import 'package:buzz/features/channels/channel_mutes/channel_mutes_storage.dart';
import 'package:buzz/features/channels/channel_stars/channel_stars_provider.dart';
import 'package:buzz/features/channels/channel_stars/channel_stars_storage.dart';
import 'package:buzz/features/channels/channel_typing_provider.dart';
import 'package:buzz/features/channels/members_sheet.dart';
import 'package:buzz/features/channels/composer_dock_size_reporter.dart';
import 'package:buzz/features/channels/date_formatters.dart';
import 'package:buzz/features/channels/day_divider.dart';
import 'package:buzz/features/channels/emoji_picker.dart';
import 'package:buzz/features/channels/ime_metrics_settle_observer.dart';
import 'package:buzz/features/channels/local_message_send_animation_provider.dart';
import 'package:buzz/features/channels/message_action_backdrop_state.dart';
import 'package:buzz/features/channels/message_actions.dart';
import 'package:buzz/features/channels/mobile_huddle_controller.dart';
import 'package:buzz/features/channels/reaction_row.dart';
import 'package:buzz/features/channels/thread_detail_page.dart';
import 'package:buzz/features/channels/thread_replies_provider.dart';
import 'package:buzz/features/channels/timeline_message.dart';
import 'package:buzz/features/channels/channels_provider.dart';
import 'package:buzz/shared/read_state/read_state_provider.dart';
import 'package:buzz/features/channels/unread_badge/observed_unread_event.dart';
import 'package:buzz/features/channels/small_avatar.dart';
import 'package:buzz/features/profile/profile_provider.dart';
import 'package:buzz/shared/profile/user_cache_provider.dart';
import 'package:buzz/shared/profile/user_profile.dart';
import 'package:buzz/features/profile/user_profile_sheet.dart';
import 'package:buzz/shared/community/community_provider.dart';
import 'package:buzz/shared/emoji/emoji_burst.dart';
import 'package:buzz/shared/mentions/agent_identity_provider.dart';
import 'package:buzz/shared/huddle/huddle.dart';
import 'package:buzz/shared/relay/relay.dart';
import 'package:buzz/shared/theme/theme.dart';
import 'package:buzz/shared/widgets/app_list_card.dart';
import 'package:buzz/shared/widgets/avatar_image.dart';
import 'package:buzz/shared/widgets/frosted_app_bar.dart';
import 'package:buzz/shared/widgets/frosted_scaffold.dart';
import 'package:buzz/shared/widgets/flapping_bee.dart';
import 'package:buzz/shared/widgets/keyboard_dismiss_on_drag.dart';
import 'package:buzz/shared/widgets/ios_glass_navigation_button.dart';
import 'package:buzz/shared/widgets/lucide_star_icon.dart';
import 'package:buzz/shared/widgets/masked_avatar_badge.dart';
import 'package:buzz/shared/widgets/skeleton.dart';
import 'package:shared_preferences/shared_preferences.dart';

const _channelId = '11111111-2222-4333-8444-555555555555';
const _huddleChannelId = '8d764100-fd8f-44cf-9c98-6d8fbd739b8c';
const _otherChannelId = '22222222-3333-4444-8555-666666666666';
const _otherHuddleChannelId = '9e875211-ae90-45df-8da9-7e9ace84ca9d';

final _mutableHuddleMembersProvider =
    NotifierProvider<_MutableHuddleMembersNotifier, List<ChannelMember>>(
      () => _MutableHuddleMembersNotifier(const []),
    );

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
  String ephemeralChannelId = _huddleChannelId,
}) => NostrEvent(
  id: id,
  pubkey: pubkey,
  createdAt: createdAt,
  kind: kind,
  tags: [
    ['h', _channelId],
  ],
  content: jsonEncode({'ephemeral_channel_id': ephemeralChannelId}),
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
  Set<String>? knownAgentPubkeys,
  Future<Set<String>> Function()? loadChannelBotPubkeys,
  bool watchChannelMembershipUpdates = false,
  Future<List<AgentDirectoryEntry>> Function()? loadAgentDirectory,
  Future<Map<String, String>> Function()? loadAgentOwners,
  UserCacheNotifier? userCacheNotifier,
  List<ChannelMember> members = const [],
  List<ChannelMember> huddleMembers = const [],
  _MutableHuddleMembersNotifier? huddleMembersNotifier,
  Channel? channel,
  List<Channel>? channels,
  _FakeChannelsNotifier? channelsNotifier,
  List<NavigatorObserver> navigatorObservers = const [],
  Future<List<ChannelMember>> Function()? loadMembers,
  List<DirectoryUser>? directoryUsers,
  ChannelActions Function(Ref ref)? createChannelActions,
  ReadStateNotifier? readStateNotifier,
  _FakeMessagesNotifier? messagesNotifier,
  _FakeTypingNotifier? typingNotifier,
  _FakeTypingNotifier? huddleTypingNotifier,
  String? canvasContent,
  String? initialMessageId,
  String? initialThreadRootId,
  InitialThreadRouteBehavior initialThreadRouteBehavior =
      InitialThreadRouteBehavior.push,
  Map<String, List<NostrEvent>> threadReplies = const {},
  Map<String, Future<List<NostrEvent>>> pendingThreadReplies = const {},
  Map<String, Future<List<NostrEvent>> Function()> threadReplyLoaders =
      const {},
  Map<String, List<NostrEvent>> localThreadReplies = const {},
  TextScaler textScaler = TextScaler.noScaling,
  bool disableAnimations = false,
  bool disableRetries = false,
  Duration? Function(int retryCount, Object error)? providerRetry,
  RelaySessionNotifier? relaySessionNotifier,
  RelayConfigNotifier? relayConfigNotifier,
  HuddleMediaFactory? huddleMediaFactory,
  HuddleTransportFactory? huddleTransportFactory,
  HuddleHumanCountLoader? huddleHumanCountLoader,
  List<NostrEvent> huddleLifecycle = const [],
  String? huddleCurrentPubkey,
  http.Client? mediaClient,
  Widget? home,
}) {
  final resolvedChannel = channel ?? _testChannel;
  final navigatorKey = GlobalKey<NavigatorState>();
  final fakeChannelsNotifier =
      channelsNotifier ?? _FakeChannelsNotifier(channels ?? [resolvedChannel]);
  final fakeMessagesNotifier =
      messagesNotifier ?? _FakeMessagesNotifier(messages);
  return ProviderScope(
    retry: providerRetry ?? (disableRetries ? (_, _) => null : null),
    overrides: [
      channelMessagesProvider(
        _channelId,
      ).overrideWith(() => fakeMessagesNotifier),
      channelTypingProvider(
        _channelId,
      ).overrideWith(() => typingNotifier ?? _FakeTypingNotifier(typing)),
      channelTypingProvider(_huddleChannelId).overrideWith(
        () =>
            huddleTypingNotifier ??
            _FakeTypingNotifier(const [], channelId: _huddleChannelId),
      ),
      userCacheProvider.overrideWith(
        () => userCacheNotifier ?? _FakeUserCacheNotifier(users),
      ),
      profileProvider.overrideWith(() => _FakeProfileNotifier()),
      channelsProvider.overrideWith(() => fakeChannelsNotifier),
      channelStarsProvider.overrideWith(_FakeChannelStarsNotifier.new),
      channelMutesProvider.overrideWith(_FakeChannelMutesNotifier.new),
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
      channelMembersProvider(_huddleChannelId).overrideWith(
        (ref) async => huddleMembersNotifier == null
            ? huddleMembers
            : ref.watch(_mutableHuddleMembersProvider),
      ),
      if (huddleMembersNotifier != null)
        _mutableHuddleMembersProvider.overrideWith(() => huddleMembersNotifier),
      if (!watchChannelMembershipUpdates)
        channelBotPubkeysProvider(_channelId).overrideWith(
          (ref) async => loadChannelBotPubkeys?.call() ?? const <String>{},
        ),
      channelBotPubkeysProvider(_huddleChannelId).overrideWith(
        (ref) async => {
          for (final member in huddleMembers)
            if (member.isBot) member.pubkey.toLowerCase(),
        },
      ),
      agentOwnersProvider.overrideWith(
        (ref) async => loadAgentOwners?.call() ?? const <String, String>{},
      ),
      agentDirectoryProvider.overrideWith(
        (ref) async => loadAgentDirectory?.call() ?? const [],
      ),
      if (knownAgentPubkeys != null)
        knownAgentPubkeysProvider.overrideWithValue(knownAgentPubkeys),
      if (directoryUsers != null)
        relayDirectoryUsersProvider.overrideWith((ref) async => directoryUsers),
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
      for (final entry in threadReplyLoaders.entries)
        threadRepliesProvider(
          ThreadRepliesArgs(channelId: _channelId, rootId: entry.key),
        ).overrideWith((ref) => entry.value()),
      for (final entry in localThreadReplies.entries)
        threadLocalRepliesProvider(
          ThreadRepliesArgs(channelId: _channelId, rootId: entry.key),
        ).overrideWith(
          () => _FakeThreadLocalRepliesNotifier(
            ThreadRepliesArgs(channelId: _channelId, rootId: entry.key),
            entry.value,
          ),
        ),
      // Stub the relay client provider so preloadMembers doesn't crash.
      relayClientProvider.overrideWithValue(
        RelayClient(baseUrl: 'http://localhost:3000'),
      ),
      if (mediaClient != null) ...[
        mediaGetAuthServiceProvider.overrideWithValue(
          MediaGetAuthService(baseUrl: 'https://relay.example', nsec: null),
        ),
        mediaHttpClientProvider.overrideWithValue(mediaClient),
      ],
      if (relaySessionNotifier != null ||
          (resolvedChannel.isDm &&
              resolvedChannel.participantPubkeys.toSet().length == 2))
        relaySessionProvider.overrideWith(
          () => relaySessionNotifier ?? _IdentityUpdateRelaySession(),
        ),
      if (relayConfigNotifier != null)
        relayConfigProvider.overrideWith(() => relayConfigNotifier),
      if (huddleMediaFactory != null)
        huddleMediaFactoryProvider.overrideWithValue(huddleMediaFactory),
      if (huddleTransportFactory != null)
        huddleTransportFactoryProvider.overrideWithValue(
          huddleTransportFactory,
        ),
      if (huddleHumanCountLoader != null)
        huddleHumanCountProvider.overrideWithValue(huddleHumanCountLoader),
      huddleLifecycleProvider(
        _channelId,
      ).overrideWith((ref) async => huddleLifecycle),
      if (huddleCurrentPubkey != null)
        currentPubkeyProvider.overrideWith((ref) => huddleCurrentPubkey),
      appLifecycleProvider.overrideWith(_TestAppLifecycleNotifier.new),
      // Compose bar drafts persist through SharedPreferences.
      savedPrefsProvider.overrideWithValue(_testPrefs),
    ],
    child: MaterialApp(
      navigatorKey: navigatorKey,
      theme: AppTheme.light(),
      builder: (context, child) => MediaQuery(
        data: MediaQuery.of(context).copyWith(
          textScaler: textScaler,
          disableAnimations: disableAnimations,
        ),
        child: MobileHuddleShell(navigatorKey: navigatorKey, child: child!),
      ),
      navigatorObservers: navigatorObservers,
      home:
          home ??
          ChannelDetailPage(
            channel: resolvedChannel,
            initialMessageId: initialMessageId,
            initialThreadRootId: initialThreadRootId,
            initialThreadRouteBehavior: initialThreadRouteBehavior,
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
      'bot-role author avatars stay squircles in channel and thread',
      (tester) async {
        final message = _textMsg(
          id: 'bot-message',
          pubkey: 'bot',
          content: 'Bot message',
        );
        await tester.pumpWidget(
          _buildTestable(
            messages: [message],
            users: const {
              'bot': UserProfile(pubkey: 'bot', displayName: 'Bot'),
            },
            loadChannelBotPubkeys: () async => const {'bot'},
            threadReplies: const {'bot-message': []},
          ),
        );
        await tester.pumpAndSettle();

        AvatarImage avatarIn(Finder row) => tester.widget<AvatarImage>(
          find.descendant(of: row, matching: find.byType(AvatarImage)),
        );
        expect(
          avatarIn(
            find.byKey(const ValueKey('message-row-bot-message')),
          ).isAgent,
          isTrue,
        );

        await tester.tap(find.byKey(const ValueKey('message-row-bot-message')));
        await tester.pumpAndSettle();
        expect(
          avatarIn(
            find.byKey(const ValueKey('thread-message-row-bot-message')),
          ).isAgent,
          isTrue,
        );
      },
    );

    testWidgets('uses the shared 32px masked presence avatar in DM headers', (
      tester,
    ) async {
      final dmChannel = Channel(
        id: _channelId,
        name: 'DM',
        channelType: 'dm',
        visibility: 'private',
        description: 'Direct message',
        createdBy: 'self',
        createdAt: DateTime(2025),
        memberCount: 2,
        participants: const ['Self', 'Alice'],
        participantPubkeys: const ['self', 'alice'],
        isMember: true,
      );

      await tester.pumpWidget(
        _buildTestable(
          messages: const [],
          channel: dmChannel,
          users: const {
            'alice': UserProfile(pubkey: 'alice', displayName: 'Alice'),
          },
        ),
      );
      await tester.pumpAndSettle();

      final avatarFinder = find.byKey(const ValueKey('dm-header-avatar'));
      final avatar = tester.widget<MaskedAvatarBadge>(avatarFinder);
      expect(tester.getSize(avatarFinder), const Size.square(32));
      expect(avatar.geometry, AvatarBadgeMaskGeometry.presenceDot);
      expect(avatar.badge, isNotNull);
      expect(
        tester
            .widget<ClipRRect>(
              find.descendant(
                of: avatarFinder,
                matching: find.byType(ClipRRect),
              ),
            )
            .borderRadius,
        BorderRadius.circular(16),
      );
      expect(
        find.descendant(of: avatarFinder, matching: find.byType(ClipPath)),
        findsOneWidget,
      );
      final name = tester.widget<Text>(
        find.byKey(const ValueKey('dm-header-name')),
      );
      final presence = tester.widget<Text>(
        find.byKey(const ValueKey('dm-header-presence')),
      );
      expect(name.style?.fontSize, 16);
      expect(name.style?.fontWeight, FontWeight.w500);
      expect(presence.style?.fontSize, 14);
      expect(presence.style?.fontWeight, FontWeight.w400);
      expect(find.byTooltip('View members'), findsNothing);
      expect(find.byTooltip('Start Huddle'), findsOneWidget);
    });

    testWidgets('uses a fallback squircle for bot-role DM participants', (
      tester,
    ) async {
      final dmChannel = Channel(
        id: _channelId,
        name: 'Bot DM',
        channelType: 'dm',
        visibility: 'private',
        description: 'Direct message with a bot',
        createdBy: 'self',
        createdAt: DateTime(2025),
        memberCount: 2,
        participants: const ['Self', 'Bot'],
        participantPubkeys: const ['self', 'bot'],
        isMember: true,
      );

      await tester.pumpWidget(
        _buildTestable(
          messages: const [],
          channel: dmChannel,
          loadChannelBotPubkeys: () async => const {'bot'},
        ),
      );
      await tester.pumpAndSettle();

      final avatarFinder = find.byKey(const ValueKey('dm-header-avatar'));
      expect(
        tester
            .widget<ClipRRect>(
              find.descendant(
                of: avatarFinder,
                matching: find.byType(ClipRRect),
              ),
            )
            .borderRadius,
        BorderRadius.circular(9.6),
      );
      expect(
        tester
            .widget<AvatarImageContent>(
              find.descendant(
                of: avatarFinder,
                matching: find.byType(AvatarImageContent),
              ),
            )
            .imageUrl,
        isNull,
      );
      expect(
        find.descendant(of: avatarFinder, matching: find.byType(ClipPath)),
        findsOneWidget,
      );
    });

    testWidgets('hides the Huddle action in a one-to-one agent DM', (
      tester,
    ) async {
      final dmChannel = Channel(
        id: _channelId,
        name: 'Agent DM',
        channelType: 'dm',
        visibility: 'private',
        description: 'Direct message with an agent',
        createdBy: 'self',
        createdAt: DateTime(2025),
        memberCount: 2,
        participants: const ['Self', 'Agent'],
        participantPubkeys: const ['self', 'agent'],
        isMember: true,
      );

      await tester.pumpWidget(
        _buildTestable(
          messages: const [],
          channel: dmChannel,
          users: const {
            'agent': UserProfile(
              pubkey: 'agent',
              displayName: 'Agent',
              ownerPubkey: 'owner',
            ),
          },
        ),
      );
      await tester.pumpAndSettle();

      final avatarFinder = find.byKey(const ValueKey('dm-header-avatar'));
      expect(
        tester
            .widget<ClipRRect>(
              find.descendant(
                of: avatarFinder,
                matching: find.byType(ClipRRect),
              ),
            )
            .borderRadius,
        BorderRadius.circular(9.6),
      );
      expect(find.byKey(const ValueKey('channel-huddle-button')), findsNothing);
      expect(find.byTooltip('Start Huddle'), findsNothing);
    });

    testWidgets('hides the Huddle action for a channel bot DM', (tester) async {
      final dmChannel = Channel(
        id: _channelId,
        name: 'Bot DM',
        channelType: 'dm',
        visibility: 'private',
        description: 'Direct message with a channel bot',
        createdBy: 'self',
        createdAt: DateTime(2025),
        memberCount: 2,
        participants: const ['Self', 'Bot'],
        participantPubkeys: const ['self', 'bot'],
        isMember: true,
      );

      await tester.pumpWidget(
        _buildTestable(
          messages: const [],
          channel: dmChannel,
          loadChannelBotPubkeys: () async => const {'bot'},
          users: const {'bot': UserProfile(pubkey: 'bot', displayName: 'Bot')},
        ),
      );
      await tester.pumpAndSettle();

      expect(find.byKey(const ValueKey('channel-huddle-button')), findsNothing);
      expect(find.byTooltip('Start Huddle'), findsNothing);
    });

    testWidgets('keeps the Huddle action hidden while agent identity loads', (
      tester,
    ) async {
      final directoryCompleter = Completer<List<AgentDirectoryEntry>>();
      final dmChannel = Channel(
        id: _channelId,
        name: 'DM',
        channelType: 'dm',
        visibility: 'private',
        description: 'Direct message',
        createdBy: 'self',
        createdAt: DateTime(2025),
        memberCount: 2,
        participants: const ['Self', 'Alice'],
        participantPubkeys: const ['self', 'alice'],
        isMember: true,
      );

      await tester.pumpWidget(
        _buildTestable(
          messages: const [],
          channel: dmChannel,
          loadAgentDirectory: () => directoryCompleter.future,
          users: const {
            'alice': UserProfile(pubkey: 'alice', displayName: 'Alice'),
          },
        ),
      );
      await tester.pump();

      expect(find.byKey(const ValueKey('channel-huddle-button')), findsNothing);
      expect(find.byTooltip('Start Huddle'), findsNothing);

      directoryCompleter.complete(const []);
      await tester.pumpAndSettle();

      expect(find.byTooltip('Start Huddle'), findsOneWidget);
    });

    testWidgets('preloads DM participant profiles without a member snapshot', (
      tester,
    ) async {
      final preloadedPubkeys = <String>[];
      final userCache = _FakeUserCacheNotifier(
        const {},
        preload: (pubkeys) async {
          preloadedPubkeys.addAll(pubkeys);
          return true;
        },
      );
      final dmChannel = Channel(
        id: _channelId,
        name: 'Human DM',
        channelType: 'dm',
        visibility: 'private',
        description: 'Direct message',
        createdBy: 'self',
        createdAt: DateTime(2025),
        memberCount: 2,
        participants: const ['Self', 'Alice'],
        participantPubkeys: const ['self', 'alice'],
        isMember: true,
      );

      await tester.pumpWidget(
        _buildTestable(
          messages: const [],
          channel: dmChannel,
          userCacheNotifier: userCache,
        ),
      );
      await tester.pumpAndSettle();

      expect(preloadedPubkeys, containsAll(const ['self', 'alice']));
      expect(find.byTooltip('Start Huddle'), findsOneWidget);
    });

    testWidgets('force-refreshes cached profiles before enabling Huddle', (
      tester,
    ) async {
      late final _FakeUserCacheNotifier userCache;
      userCache = _FakeUserCacheNotifier(
        const {
          'agent': UserProfile(pubkey: 'agent', displayName: 'Cached Human'),
        },
        preload: (_) async {
          await Future<void>.delayed(Duration.zero);
          userCache.replace(
            const UserProfile(
              pubkey: 'agent',
              displayName: 'Agent',
              ownerPubkey: 'owner',
            ),
          );
          return true;
        },
      );
      final dmChannel = Channel(
        id: _channelId,
        name: 'Agent DM',
        channelType: 'dm',
        visibility: 'private',
        description: 'Direct message',
        createdBy: 'self',
        createdAt: DateTime(2025),
        memberCount: 2,
        participants: const ['Self', 'Agent'],
        participantPubkeys: const ['self', 'agent'],
        isMember: true,
      );

      await tester.pumpWidget(
        _buildTestable(
          messages: const [],
          channel: dmChannel,
          userCacheNotifier: userCache,
        ),
      );
      await tester.pumpAndSettle();

      expect(userCache.state['agent']?.ownerPubkey, 'owner');
      expect(find.byTooltip('Start Huddle'), findsNothing);
    });

    testWidgets('keeps Huddle hidden when live owner profile beats refresh', (
      tester,
    ) async {
      final owner = nostr.Keys.generate();
      final agent = nostr.Keys.generate();
      final profileRefresh = Completer<List<NostrEvent>>();
      final relaySession = _IdentityUpdateRelaySession(
        profileRefresh: profileRefresh.future,
      );
      final userCache = UserCacheNotifier();
      final dmChannel = Channel(
        id: _channelId,
        name: 'Agent DM',
        channelType: 'dm',
        visibility: 'private',
        description: 'Direct message',
        createdBy: 'self',
        createdAt: DateTime(2025),
        memberCount: 2,
        participants: const ['Self', 'Agent'],
        participantPubkeys: ['self', agent.public],
        isMember: true,
      );

      await tester.pumpWidget(
        _buildTestable(
          messages: const [],
          channel: dmChannel,
          userCacheNotifier: userCache,
          relaySessionNotifier: relaySession,
          members: [
            ChannelMember(
              pubkey: 'self',
              role: 'member',
              joinedAt: DateTime(2025),
            ),
            ChannelMember(
              pubkey: agent.public,
              role: 'member',
              joinedAt: DateTime(2025),
            ),
          ],
        ),
      );
      await tester.pump();
      expect(find.byTooltip('Start Huddle'), findsNothing);

      relaySession.emitProfile(
        _profileEvent(
          id: 'newer-agent',
          pubkey: agent.public,
          createdAt: 2,
          name: 'Agent',
          tags: [_authTag(owner, agent.public)],
        ),
      );
      profileRefresh.complete([
        _profileEvent(
          id: 'older-human',
          pubkey: agent.public,
          createdAt: 1,
          name: 'Human',
        ),
      ]);
      await tester.pumpAndSettle();

      expect(userCache.state[agent.public]?.ownerPubkey, owner.public);
      expect(find.byTooltip('Start Huddle'), findsNothing);
    });

    testWidgets('keeps Huddle hidden while a verified owner profile loads', (
      tester,
    ) async {
      final profilePreloadCompleter = Completer<bool>();
      final userCache = _FakeUserCacheNotifier(
        const {},
        preload: (_) => profilePreloadCompleter.future,
      );
      final dmChannel = Channel(
        id: _channelId,
        name: 'Agent DM',
        channelType: 'dm',
        visibility: 'private',
        description: 'Direct message',
        createdBy: 'self',
        createdAt: DateTime(2025),
        memberCount: 2,
        participants: const ['Self', 'Agent'],
        participantPubkeys: const ['self', 'agent'],
        isMember: true,
      );

      await tester.pumpWidget(
        _buildTestable(
          messages: const [],
          channel: dmChannel,
          userCacheNotifier: userCache,
          members: [
            ChannelMember(
              pubkey: 'self',
              role: 'member',
              joinedAt: DateTime(2025),
            ),
            ChannelMember(
              pubkey: 'agent',
              role: 'member',
              joinedAt: DateTime(2025),
            ),
          ],
        ),
      );
      await tester.pump();

      expect(find.byTooltip('Start Huddle'), findsNothing);

      userCache.replace(
        const UserProfile(
          pubkey: 'agent',
          displayName: 'Agent',
          ownerPubkey: 'owner',
        ),
      );
      profilePreloadCompleter.complete(true);
      await tester.pumpAndSettle();

      expect(find.byTooltip('Start Huddle'), findsNothing);
    });

    testWidgets('rechecks verified owner profiles after reconnect', (
      tester,
    ) async {
      final relaySession = _IdentityUpdateRelaySession();
      final reconnectPreloadCompleter = Completer<bool>();
      var memberPreloadCount = 0;
      var blockMemberPreload = false;
      final userCache = _FakeUserCacheNotifier(
        const {},
        preload: (pubkeys) {
          if (pubkeys.length == 1) return Future.value(true);
          memberPreloadCount++;
          return blockMemberPreload
              ? reconnectPreloadCompleter.future
              : Future.value(true);
        },
      );
      final dmChannel = Channel(
        id: _channelId,
        name: 'Agent DM',
        channelType: 'dm',
        visibility: 'private',
        description: 'Direct message',
        createdBy: 'self',
        createdAt: DateTime(2025),
        memberCount: 2,
        participants: const ['Self', 'Agent'],
        participantPubkeys: const ['self', 'agent'],
        isMember: true,
      );

      await tester.pumpWidget(
        _buildTestable(
          messages: const [],
          channel: dmChannel,
          userCacheNotifier: userCache,
          relaySessionNotifier: relaySession,
          members: [
            ChannelMember(
              pubkey: 'self',
              role: 'member',
              joinedAt: DateTime(2025),
            ),
            ChannelMember(
              pubkey: 'agent',
              role: 'member',
              joinedAt: DateTime(2025),
            ),
          ],
        ),
      );
      await tester.pumpAndSettle();

      expect(find.byTooltip('Start Huddle'), findsOneWidget);
      final memberPreloadsBeforeReconnect = memberPreloadCount;
      blockMemberPreload = true;

      relaySession.disconnect();
      await tester.pump();
      expect(find.byTooltip('Start Huddle'), findsNothing);

      relaySession.connect();
      await tester.pump();
      expect(memberPreloadCount, greaterThan(memberPreloadsBeforeReconnect));
      expect(find.byTooltip('Start Huddle'), findsNothing);

      userCache.replace(
        const UserProfile(
          pubkey: 'agent',
          displayName: 'Agent',
          ownerPubkey: 'owner',
        ),
      );
      reconnectPreloadCompleter.complete(true);
      await tester.pumpAndSettle();

      expect(find.byTooltip('Start Huddle'), findsNothing);
      await tester.pump(const Duration(milliseconds: 500));
    });

    testWidgets('keeps directory-only agent Huddle hidden after disconnect', (
      tester,
    ) async {
      final relaySession = _IdentityUpdateRelaySession();
      final dmChannel = Channel(
        id: _channelId,
        name: 'Agent DM',
        channelType: 'dm',
        visibility: 'private',
        description: 'Direct message',
        createdBy: 'self',
        createdAt: DateTime(2025),
        memberCount: 2,
        participants: const ['Self', 'Agent'],
        participantPubkeys: const ['self', 'agent'],
        isMember: true,
      );

      await tester.pumpWidget(
        _buildTestable(
          messages: const [],
          channel: dmChannel,
          relaySessionNotifier: relaySession,
          loadAgentDirectory: () async => const [
            AgentDirectoryEntry(pubkey: 'agent'),
          ],
          members: [
            ChannelMember(
              pubkey: 'self',
              role: 'member',
              joinedAt: DateTime(2025),
            ),
            ChannelMember(
              pubkey: 'agent',
              role: 'member',
              joinedAt: DateTime(2025),
            ),
          ],
        ),
      );
      await tester.pumpAndSettle();

      expect(find.byTooltip('Start Huddle'), findsNothing);

      relaySession.disconnect();
      await tester.pumpAndSettle();

      expect(find.byTooltip('Start Huddle'), findsNothing);
    });

    testWidgets('keeps bot-role-only Huddle hidden after disconnect', (
      tester,
    ) async {
      final relaySession = _IdentityUpdateRelaySession();
      final dmChannel = Channel(
        id: _channelId,
        name: 'Bot DM',
        channelType: 'dm',
        visibility: 'private',
        description: 'Direct message',
        createdBy: 'self',
        createdAt: DateTime(2025),
        memberCount: 2,
        participants: const ['Self', 'Bot'],
        participantPubkeys: const ['self', 'bot'],
        isMember: true,
      );

      await tester.pumpWidget(
        _buildTestable(
          messages: const [],
          channel: dmChannel,
          relaySessionNotifier: relaySession,
          loadChannelBotPubkeys: () async => const {'bot'},
          members: [
            ChannelMember(
              pubkey: 'self',
              role: 'member',
              joinedAt: DateTime(2025),
            ),
            ChannelMember(
              pubkey: 'bot',
              role: 'member',
              joinedAt: DateTime(2025),
            ),
          ],
        ),
      );
      await tester.pumpAndSettle();

      expect(find.byTooltip('Start Huddle'), findsNothing);

      relaySession.disconnect();
      await tester.pumpAndSettle();

      expect(find.byTooltip('Start Huddle'), findsNothing);
    });

    testWidgets('keeps Huddle hidden until bot-role replay reaches EOSE', (
      tester,
    ) async {
      final relaySession = _IdentityUpdateRelaySession();
      final dmChannel = Channel(
        id: _channelId,
        name: 'Human DM',
        channelType: 'dm',
        visibility: 'private',
        description: 'Direct message',
        createdBy: 'self',
        createdAt: DateTime(2025),
        memberCount: 2,
        participants: const ['Self', 'Alice'],
        participantPubkeys: const ['self', 'alice'],
        isMember: true,
      );

      await tester.pumpWidget(
        _buildTestable(
          messages: const [],
          channel: dmChannel,
          relaySessionNotifier: relaySession,
          watchChannelMembershipUpdates: true,
          members: [
            ChannelMember(
              pubkey: 'self',
              role: 'member',
              joinedAt: DateTime(2025),
            ),
            ChannelMember(
              pubkey: 'alice',
              role: 'member',
              joinedAt: DateTime(2025),
            ),
          ],
        ),
      );
      await tester.pumpAndSettle();
      expect(find.byTooltip('Start Huddle'), findsOneWidget);

      relaySession.beginMembershipReplay();
      await tester.pump();
      expect(find.byTooltip('Start Huddle'), findsNothing);

      relaySession.emitReplayedMembership(
        NostrEvent(
          id: 'membership-self',
          pubkey: 'relay',
          createdAt: 1,
          kind: 39002,
          tags: const [
            ['d', _channelId],
            ['p', 'self'],
          ],
          content: '',
          sig: 'sig',
        ),
      );
      await tester.pumpAndSettle();
      expect(find.byTooltip('Start Huddle'), findsNothing);

      relaySession.emitReplayedMembership(
        NostrEvent(
          id: 'membership-bot',
          pubkey: 'relay',
          createdAt: 2,
          kind: 39002,
          tags: const [
            ['d', _channelId],
            ['p', 'self'],
            ['p', 'alice', '', 'bot'],
          ],
          content: '',
          sig: 'sig',
        ),
      );
      await tester.pumpAndSettle();
      expect(find.byTooltip('Start Huddle'), findsNothing);

      relaySession.finishMembershipReplay();
      await tester.pumpAndSettle();
      expect(find.byTooltip('Start Huddle'), findsNothing);
    });

    testWidgets('keeps the Huddle action hidden when identity loading fails', (
      tester,
    ) async {
      final dmChannel = Channel(
        id: _channelId,
        name: 'Agent DM',
        channelType: 'dm',
        visibility: 'private',
        description: 'Direct message',
        createdBy: 'self',
        createdAt: DateTime(2025),
        memberCount: 2,
        participants: const ['Self', 'Agent'],
        participantPubkeys: const ['self', 'agent'],
        isMember: true,
      );

      await tester.pumpWidget(
        _buildTestable(
          messages: const [],
          channel: dmChannel,
          loadAgentOwners: () => Future.error('identity unavailable'),
          disableRetries: true,
        ),
      );
      await tester.pumpAndSettle();

      expect(find.byTooltip('Start Huddle'), findsNothing);
    });

    testWidgets('keeps the Huddle action hidden when member preload fails', (
      tester,
    ) async {
      final dmChannel = Channel(
        id: _channelId,
        name: 'Agent DM',
        channelType: 'dm',
        visibility: 'private',
        description: 'Direct message',
        createdBy: 'self',
        createdAt: DateTime(2025),
        memberCount: 2,
        participants: const ['Self', 'Agent'],
        participantPubkeys: const ['self', 'agent'],
        isMember: true,
      );

      await tester.pumpWidget(
        _buildTestable(
          messages: const [],
          channel: dmChannel,
          loadMembers: () => Future.error('members unavailable'),
          disableRetries: true,
        ),
      );
      await tester.pumpAndSettle();

      expect(find.byTooltip('Start Huddle'), findsNothing);
    });

    testWidgets('hides Huddle when a participant becomes an agent live', (
      tester,
    ) async {
      final relaySession = _IdentityUpdateRelaySession();
      final dmChannel = Channel(
        id: _channelId,
        name: 'Human DM',
        channelType: 'dm',
        visibility: 'private',
        description: 'Direct message',
        createdBy: 'self',
        createdAt: DateTime(2025),
        memberCount: 2,
        participants: const ['Self', 'Alice'],
        participantPubkeys: const ['self', 'alice'],
        isMember: true,
      );

      var directoryLoadCount = 0;
      await tester.pumpWidget(
        _buildTestable(
          messages: const [],
          channel: dmChannel,
          relaySessionNotifier: relaySession,
          loadAgentDirectory: () async {
            directoryLoadCount++;
            return directoryLoadCount == 1
                ? const []
                : const [AgentDirectoryEntry(pubkey: 'alice')];
          },
          members: [
            ChannelMember(
              pubkey: 'self',
              role: 'member',
              joinedAt: DateTime(2025),
            ),
            ChannelMember(
              pubkey: 'alice',
              role: 'member',
              joinedAt: DateTime(2025),
            ),
          ],
        ),
      );
      await tester.pumpAndSettle();

      expect(relaySession.identityFilter?.kinds, const [0, 10100]);
      expect(relaySession.identityFilter?.authors, contains('alice'));
      expect(relaySession.identityFilter?.limit, 100);
      expect(find.byTooltip('Start Huddle'), findsOneWidget);

      relaySession.emitAgentProfile(pubkey: 'alice');
      await tester.pumpAndSettle();

      expect(find.byTooltip('Start Huddle'), findsNothing);
    });

    testWidgets('keeps Huddle hidden while identity replay retries', (
      tester,
    ) async {
      final relaySession = _IdentityUpdateRelaySession();
      final dmChannel = Channel(
        id: _channelId,
        name: 'Human DM',
        channelType: 'dm',
        visibility: 'private',
        description: 'Direct message',
        createdBy: 'self',
        createdAt: DateTime(2025),
        memberCount: 2,
        participants: const ['Self', 'Alice'],
        participantPubkeys: const ['self', 'alice'],
        isMember: true,
      );

      await tester.pumpWidget(
        _buildTestable(
          messages: const [],
          channel: dmChannel,
          relaySessionNotifier: relaySession,
          members: [
            ChannelMember(
              pubkey: 'self',
              role: 'member',
              joinedAt: DateTime(2025),
            ),
            ChannelMember(
              pubkey: 'alice',
              role: 'member',
              joinedAt: DateTime(2025),
            ),
          ],
        ),
      );
      await tester.pumpAndSettle();
      expect(find.byTooltip('Start Huddle'), findsOneWidget);

      relaySession.retryIdentitySubscription();
      await tester.pump();
      expect(find.byTooltip('Start Huddle'), findsNothing);

      relaySession.emitAgentProfile(pubkey: 'alice');
      await tester.pumpAndSettle();
      expect(find.byTooltip('Start Huddle'), findsNothing);

      relaySession.readyIdentitySubscription();
      await tester.pumpAndSettle();
      expect(find.byTooltip('Start Huddle'), findsNothing);
    });

    testWidgets('queries DM participants directly for agent identity', (
      tester,
    ) async {
      final relaySession = _IdentityUpdateRelaySession();
      final dmChannel = Channel(
        id: _channelId,
        name: 'Human DM',
        channelType: 'dm',
        visibility: 'private',
        description: 'Direct message',
        createdBy: 'self',
        createdAt: DateTime(2025),
        memberCount: 2,
        participants: const ['Self', 'Alice'],
        participantPubkeys: const ['self', 'alice'],
        isMember: true,
      );

      await tester.pumpWidget(
        _buildTestable(
          messages: const [],
          channel: dmChannel,
          relaySessionNotifier: relaySession,
          members: [
            ChannelMember(
              pubkey: 'self',
              role: 'member',
              joinedAt: DateTime(2025),
            ),
            ChannelMember(
              pubkey: 'alice',
              role: 'member',
              joinedAt: DateTime(2025),
            ),
          ],
        ),
      );
      await tester.pumpAndSettle();

      expect(relaySession.directIdentityFilter?.kinds, const [10100]);
      expect(
        relaySession.directIdentityFilter?.authors,
        containsAll(const ['self', 'alice']),
      );
      expect(relaySession.directIdentityFilter?.limit, 2);
    });

    testWidgets('hides Huddle for an agent found by direct DM lookup', (
      tester,
    ) async {
      final relaySession = _IdentityUpdateRelaySession()
        ..directIdentityProfiles = const [
          NostrEvent(
            id: 'old-agent-profile',
            pubkey: 'alice',
            createdAt: 1,
            kind: 10100,
            tags: [],
            content: '{"name":"Agent"}',
            sig: 'sig',
          ),
        ];
      final dmChannel = Channel(
        id: _channelId,
        name: 'Agent DM',
        channelType: 'dm',
        visibility: 'private',
        description: 'Direct message',
        createdBy: 'self',
        createdAt: DateTime(2025),
        memberCount: 2,
        participants: const ['Self', 'Alice'],
        participantPubkeys: const ['self', 'alice'],
        isMember: true,
      );

      await tester.pumpWidget(
        _buildTestable(
          messages: const [],
          channel: dmChannel,
          relaySessionNotifier: relaySession,
          members: [
            ChannelMember(
              pubkey: 'self',
              role: 'member',
              joinedAt: DateTime(2025),
            ),
            ChannelMember(
              pubkey: 'alice',
              role: 'member',
              joinedAt: DateTime(2025),
            ),
          ],
        ),
      );
      await tester.pumpAndSettle();

      expect(find.byTooltip('Start Huddle'), findsNothing);
    });

    testWidgets(
      'keeps Huddle hidden if the live identity subscription closes',
      (tester) async {
        final relaySession = _IdentityUpdateRelaySession();
        final dmChannel = Channel(
          id: _channelId,
          name: 'Human DM',
          channelType: 'dm',
          visibility: 'private',
          description: 'Direct message',
          createdBy: 'self',
          createdAt: DateTime(2025),
          memberCount: 2,
          participants: const ['Self', 'Alice'],
          participantPubkeys: const ['self', 'alice'],
          isMember: true,
        );

        await tester.pumpWidget(
          _buildTestable(
            messages: const [],
            channel: dmChannel,
            relaySessionNotifier: relaySession,
            members: [
              ChannelMember(
                pubkey: 'self',
                role: 'member',
                joinedAt: DateTime(2025),
              ),
              ChannelMember(
                pubkey: 'alice',
                role: 'member',
                joinedAt: DateTime(2025),
              ),
            ],
          ),
        );
        await tester.pumpAndSettle();

        expect(find.byTooltip('Start Huddle'), findsOneWidget);

        relaySession.closeIdentitySubscription();
        await tester.pump();

        expect(find.byTooltip('Start Huddle'), findsNothing);
      },
    );

    testWidgets('keeps the Members action for group DMs', (tester) async {
      final dmChannel = Channel(
        id: _channelId,
        name: 'DM',
        channelType: 'dm',
        visibility: 'private',
        description: 'Group direct message',
        createdBy: 'self',
        createdAt: DateTime(2025),
        memberCount: 3,
        participants: const ['Self', 'Alice', 'Bob'],
        participantPubkeys: const ['self', 'alice', 'bob'],
        isMember: true,
      );

      await tester.pumpWidget(
        _buildTestable(
          messages: const [],
          channel: dmChannel,
          knownAgentPubkeys: const {'alice'},
          users: const {
            'alice': UserProfile(pubkey: 'alice', displayName: 'Alice'),
            'bob': UserProfile(pubkey: 'bob', displayName: 'Bob'),
          },
        ),
      );
      await tester.pumpAndSettle();

      expect(find.byTooltip('View members'), findsOneWidget);
      expect(find.byTooltip('Start Huddle'), findsOneWidget);
    });

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
      final scaffold = tester.widget<FrostedScaffold>(
        find.byType(FrostedScaffold).first,
      );
      expect(scaffold.resizeToAvoidBottomInset, isTrue);
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

    testWidgets(
      'channel details combines people and agents in one member list',
      (tester) async {
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
              ChannelMember(
                pubkey: 'agent',
                role: 'bot',
                joinedAt: DateTime(2025),
                displayName: 'Agent',
              ),
            ],
          ),
        );
        await tester.pumpAndSettle();

        await tester.tap(
          find.byKey(const ValueKey('channel-header-settings-trigger')),
        );
        await tester.pumpAndSettle();
        expect(find.text('3 members'), findsOneWidget);
        expect(find.text('You · Owner', findRichText: true), findsOneWidget);
        expect(find.text('Alice · Member', findRichText: true), findsOneWidget);
        expect(find.text('Agent · Agent', findRichText: true), findsOneWidget);
        expect(find.text('Member'), findsNothing);
        expect(find.text('Owner'), findsNothing);
        expect(find.text('People · 2'), findsNothing);
        expect(find.text('Agents · 1'), findsNothing);
        expect(find.text('PEOPLE — 2'), findsNothing);
        expect(find.text('BOTS — 1'), findsNothing);

        final aliceRow = find.byKey(
          const ValueKey('channel-details-member-alice'),
        );
        final aliceText = tester.widget<Text>(
          find.descendant(
            of: aliceRow,
            matching: find.byWidgetPredicate(
              (widget) =>
                  widget is Text &&
                  widget.textSpan?.toPlainText() == 'Alice · Member',
            ),
          ),
        );
        final aliceSpans = (aliceText.textSpan! as TextSpan).children!;
        expect(
          aliceSpans.last.style?.fontSize,
          AppTheme.light().textTheme.bodySmall?.fontSize,
        );
        expect(
          tester
              .widget<AvatarImage>(
                find.descendant(
                  of: aliceRow,
                  matching: find.byType(AvatarImage),
                ),
              )
              .radius,
          20,
        );
        expect(
          find.descendant(
            of: aliceRow,
            matching: find.byIcon(LucideIcons.chevronRight),
          ),
          findsOneWidget,
        );
        expect(tester.getSize(aliceRow).height, 40 + (Grid.xxs * 2));

        await tester.tap(aliceRow);
        await tester.pumpAndSettle();
        expect(find.byType(UserProfileSheet), findsOneWidget);
      },
    );

    testWidgets('hides Add members while the authoritative roster is loading', (
      tester,
    ) async {
      final members = Completer<List<ChannelMember>>();
      await tester.pumpWidget(
        _buildTestable(messages: const [], loadMembers: () => members.future),
      );
      await tester.pump();

      await tester.tap(
        find.byKey(const ValueKey('channel-header-settings-trigger')),
      );
      await tester.pump(const Duration(milliseconds: 500));

      expect(
        find.byKey(const ValueKey('channel-details-add-members-row')),
        findsNothing,
      );

      members.complete([
        ChannelMember(pubkey: 'self', role: 'owner', joinedAt: DateTime(2025)),
      ]);
      await tester.pumpAndSettle();
      expect(
        find.byKey(const ValueKey('channel-details-add-members-row')),
        findsOneWidget,
      );
    });

    testWidgets('hides Add members when the authoritative roster fails', (
      tester,
    ) async {
      await tester.pumpWidget(
        _buildTestable(
          messages: const [],
          loadMembers: () => Future<List<ChannelMember>>.error('failed'),
        ),
      );
      await tester.pump();
      await tester.pumpAndSettle();

      await tester.tap(
        find.byKey(const ValueKey('channel-header-settings-trigger')),
      );
      await tester.pumpAndSettle();

      expect(
        find.byKey(const ValueKey('channel-details-add-members-row')),
        findsNothing,
      );
      expect(find.text('Members unavailable'), findsOneWidget);
    });

    testWidgets('small channel keeps member administration reachable', (
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

      await tester.tap(
        find.byKey(const ValueKey('channel-header-settings-trigger')),
      );
      await tester.pumpAndSettle();
      final addRow = find.byKey(
        const ValueKey('channel-details-add-members-row'),
      );
      final memberRow = find.byKey(
        const ValueKey('channel-details-member-self'),
      );
      final seeAllRow = find.byKey(
        const ValueKey('channel-details-members-row'),
      );
      expect(addRow, findsOneWidget);
      expect(memberRow, findsOneWidget);
      expect(seeAllRow, findsOneWidget);
      expect(
        tester.widget<Text>(find.text('Add members')).style,
        Theme.of(tester.element(addRow)).textTheme.bodyLarge,
      );
      expect(
        tester.getTopLeft(addRow).dy,
        lessThan(tester.getTopLeft(memberRow).dy),
      );

      await tester.ensureVisible(seeAllRow);
      await tester.pumpAndSettle();
      await tester.tap(seeAllRow);
      await tester.pumpAndSettle();
      expect(find.byType(MembersSheet), findsOneWidget);

      await tester.tap(find.byIcon(LucideIcons.ellipsis));
      await tester.pumpAndSettle();
      expect(find.text('Role'), findsOneWidget);
      expect(find.text('Remove from channel'), findsOneWidget);
    });

    testWidgets('action tiles expose button and enabled semantics', (
      tester,
    ) async {
      await tester.pumpWidget(_buildTestable(messages: const []));
      await tester.pumpAndSettle();

      await tester.tap(
        find.byKey(const ValueKey('channel-header-settings-trigger')),
      );
      await tester.pumpAndSettle();

      final starSemantics = tester.getSemantics(
        find.byKey(const ValueKey('channel-details-star-action')),
      );
      expect(starSemantics.label, 'Star');
      expect(starSemantics.flagsCollection.isButton, isTrue);
      expect(
        starSemantics.flagsCollection.isEnabled.toString(),
        'Tristate.isTrue',
      );
      expect(
        starSemantics.getSemanticsData().hasAction(SemanticsAction.tap),
        isTrue,
      );

      final editSemantics = tester.getSemantics(
        find.byKey(const ValueKey('channel-details-edit-action')),
      );
      expect(editSemantics.label, 'Edit');
      expect(editSemantics.flagsCollection.isButton, isTrue);
      expect(
        editSemantics.flagsCollection.isEnabled.toString(),
        'Tristate.isFalse',
      );
      expect(
        editSemantics.getSemanticsData().hasAction(SemanticsAction.tap),
        isFalse,
      );
    });

    testWidgets(
      'star and mute actions update their visible state immediately',
      (tester) async {
        await tester.pumpWidget(_buildTestable(messages: const []));
        await tester.pumpAndSettle();

        await tester.tap(
          find.byKey(const ValueKey('channel-header-settings-trigger')),
        );
        await tester.pumpAndSettle();

        final starAction = find.byKey(
          const ValueKey('channel-details-star-action'),
        );
        final muteAction = find.byKey(
          const ValueKey('channel-details-mute-action'),
        );
        expect(find.text('Star'), findsOneWidget);
        var star = tester.widget<LucideStarIcon>(find.byType(LucideStarIcon));
        expect(star.filled, isFalse);
        expect(find.text('Mute'), findsOneWidget);
        expect(find.byIcon(LucideIcons.bellOff), findsOneWidget);

        await tester.tap(starAction);
        await tester.pump();

        expect(find.text('Unstar'), findsOneWidget);
        star = tester.widget<LucideStarIcon>(find.byType(LucideStarIcon));
        expect(star.filled, isTrue);
        expect(star.color, AppTheme.light().colorScheme.primary);

        await tester.tap(muteAction);
        await tester.pump();

        expect(find.text('Unmute'), findsOneWidget);
        final activeBell = tester.widget<Icon>(find.byIcon(LucideIcons.bell));
        expect(activeBell.color, AppTheme.light().colorScheme.primary);

        await tester.tap(starAction);
        await tester.tap(muteAction);
        await tester.pump();

        expect(find.text('Star'), findsOneWidget);
        expect(find.text('Mute'), findsOneWidget);
        star = tester.widget<LucideStarIcon>(find.byType(LucideStarIcon));
        expect(star.filled, isFalse);
        expect(find.byIcon(LucideIcons.bellOff), findsOneWidget);
      },
    );

    testWidgets('action tiles grow together for large text on a narrow page', (
      tester,
    ) async {
      tester.view.physicalSize = const Size(320, 640);
      tester.view.devicePixelRatio = 1;
      addTearDown(tester.view.resetPhysicalSize);
      addTearDown(tester.view.resetDevicePixelRatio);

      await tester.pumpWidget(
        _buildTestable(
          messages: const [],
          textScaler: const TextScaler.linear(2),
        ),
      );
      await tester.pumpAndSettle();

      await tester.tap(
        find.byKey(const ValueKey('channel-header-settings-trigger')),
      );
      await tester.pumpAndSettle();

      final starAction = find.byKey(
        const ValueKey('channel-details-star-action'),
      );
      final muteAction = find.byKey(
        const ValueKey('channel-details-mute-action'),
      );
      final editAction = find.byKey(
        const ValueKey('channel-details-edit-action'),
      );
      expect(find.text('Star'), findsOneWidget);
      expect(find.text('Mute'), findsOneWidget);
      expect(find.text('Edit'), findsOneWidget);
      expect(tester.getSize(starAction).height, greaterThan(84));
      expect(
        tester.getSize(muteAction).height,
        tester.getSize(starAction).height,
      );
      expect(
        tester.getSize(editAction).height,
        tester.getSize(starAction).height,
      );
      expect(tester.takeException(), isNull);
    });

    testWidgets('previews five members before an icon-free See all row', (
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
            ),
            for (var index = 0; index < 5; index++)
              ChannelMember(
                pubkey: 'member-$index',
                role: 'member',
                joinedAt: DateTime(2025),
              ),
            ChannelMember(
              pubkey: 'agent',
              role: 'bot',
              joinedAt: DateTime(2025),
            ),
          ],
        ),
      );
      await tester.pumpAndSettle();

      await tester.tap(
        find.byKey(const ValueKey('channel-header-settings-trigger')),
      );
      await tester.pumpAndSettle();

      final previews = find.byWidgetPredicate((widget) {
        final key = widget.key;
        return key is ValueKey<String> &&
            key.value.startsWith('channel-details-member-');
      });
      expect(previews, findsNWidgets(5));
      expect(find.text('See all'), findsOneWidget);
      final seeAllRow = find.byKey(
        const ValueKey('channel-details-members-row'),
      );
      expect(seeAllRow, findsOneWidget);
      expect(
        tester.widget<Text>(find.text('See all')).style,
        Theme.of(tester.element(seeAllRow)).textTheme.bodyLarge,
      );
      expect(
        find.descendant(of: seeAllRow, matching: find.text('7 members')),
        findsNothing,
      );
      expect(
        find.descendant(
          of: seeAllRow,
          matching: find.byIcon(LucideIcons.users),
        ),
        findsNothing,
      );
      final firstMemberRow = previews.first;
      final firstMemberTitle = find.descendant(
        of: firstMemberRow,
        matching: find.byWidgetPredicate(
          (widget) => widget is Text && widget.textSpan != null,
        ),
      );
      expect(
        tester.getTopLeft(find.text('See all')).dx,
        closeTo(tester.getTopLeft(firstMemberTitle).dx, 0.1),
      );
      expect(tester.getSize(seeAllRow).height, 40 + (Grid.xxs * 2));

      await tester.ensureVisible(seeAllRow);
      await tester.pumpAndSettle();
      await tester.tap(seeAllRow);
      await tester.pumpAndSettle();
      await tester.drag(
        find.byKey(const ValueKey('members-sheet-list')),
        const Offset(0, -500),
      );
      await tester.pumpAndSettle();
      expect(find.text('Agent'), findsOneWidget);
      expect(find.text('Bot'), findsNothing);
    });

    testWidgets('Add members keeps its close control below the safe top', (
      tester,
    ) async {
      tester.view.physicalSize = const Size(390, 844);
      tester.view.devicePixelRatio = 1;
      tester.view.viewPadding = const FakeViewPadding(top: 47, bottom: 34);
      tester.view.viewInsets = const FakeViewPadding(bottom: 300);
      addTearDown(tester.view.resetPhysicalSize);
      addTearDown(tester.view.resetDevicePixelRatio);
      addTearDown(tester.view.resetViewPadding);
      addTearDown(tester.view.resetViewInsets);

      await tester.pumpWidget(
        _buildTestable(
          messages: const [],
          members: [
            ChannelMember(
              pubkey: 'self',
              role: 'owner',
              joinedAt: DateTime(2025),
            ),
          ],
          directoryUsers: const [
            DirectoryUser(pubkey: 'alice', displayName: 'Alice'),
          ],
        ),
      );
      await tester.pumpAndSettle();

      await tester.tap(
        find.byKey(const ValueKey('channel-header-settings-trigger')),
      );
      await tester.pumpAndSettle();
      await tester.tap(
        find.byKey(const ValueKey('channel-details-add-members-row')),
      );
      await tester.pumpAndSettle();

      final sheet = find.byType(BottomSheet).last;
      final closeButton = find.byTooltip('Close sheet');
      expect(closeButton, findsOneWidget);
      expect(tester.getTopLeft(sheet).dy, greaterThanOrEqualTo(47 + Grid.xs));
      expect(tester.getRect(closeButton).bottom, lessThan(844 - 300));
    });

    testWidgets('Add members submits selected directory users', (tester) async {
      List<String>? addedPubkeys;
      await tester.pumpWidget(
        _buildTestable(
          messages: const [],
          members: [
            ChannelMember(
              pubkey: 'self',
              role: 'owner',
              joinedAt: DateTime(2025),
            ),
          ],
          directoryUsers: const [
            DirectoryUser(pubkey: 'alice', displayName: 'Alice'),
          ],
          createChannelActions: (ref) => _FakeChannelActions(
            ref,
            onAddMembers: (_, pubkeys) async => addedPubkeys = pubkeys,
          ),
        ),
      );
      await tester.pumpAndSettle();

      await tester.tap(
        find.byKey(const ValueKey('channel-header-settings-trigger')),
      );
      await tester.pumpAndSettle();
      await tester.tap(
        find.byKey(const ValueKey('channel-details-add-members-row')),
      );
      await tester.pumpAndSettle();

      await tester.pump();
      final unselectedSemantics = tester.getSemantics(
        find.byKey(const ValueKey('add-channel-member-alice')),
      );
      expect(unselectedSemantics.flagsCollection.isButton, isTrue);
      expect(
        unselectedSemantics.flagsCollection.isSelected.toString(),
        'Tristate.isFalse',
      );

      await tester.tap(find.byKey(const ValueKey('add-channel-member-alice')));
      await tester.pump();
      final selectedSemantics = tester.getSemantics(
        find.byKey(const ValueKey('add-channel-member-alice')),
      );
      expect(selectedSemantics.flagsCollection.isButton, isTrue);
      expect(
        selectedSemantics.flagsCollection.isSelected.toString(),
        'Tristate.isTrue',
      );
      expect(selectedSemantics.label, 'Alice, selected');
      await tester.tap(
        find.byKey(const ValueKey('add-channel-members-submit')),
      );
      await tester.pumpAndSettle();

      expect(addedPubkeys, ['alice']);
      expect(
        find.byKey(const ValueKey('add-channel-members-search')),
        findsNothing,
      );
    });

    testWidgets(
      'keeps only rejected members selected after a partial failure',
      (tester) async {
        var attempts = 0;
        final submittedPubkeys = <List<String>>[];
        await tester.pumpWidget(
          _buildTestable(
            messages: const [],
            members: [
              ChannelMember(
                pubkey: 'self',
                role: 'owner',
                joinedAt: DateTime(2025),
              ),
            ],
            directoryUsers: const [
              DirectoryUser(pubkey: 'alice', displayName: 'Alice'),
              DirectoryUser(pubkey: 'bob', displayName: 'Bob'),
            ],
            createChannelActions: (ref) => _FakeChannelActions(
              ref,
              onAddMembers: (_, pubkeys) async {
                submittedPubkeys.add(pubkeys);
                attempts += 1;
                if (attempts == 1) {
                  throw const AddMembersException({'bob': 'rejected'});
                }
              },
            ),
          ),
        );
        await tester.pumpAndSettle();

        await tester.tap(
          find.byKey(const ValueKey('channel-header-settings-trigger')),
        );
        await tester.pumpAndSettle();
        await tester.tap(
          find.byKey(const ValueKey('channel-details-add-members-row')),
        );
        await tester.pumpAndSettle();

        await tester.tap(
          find.byKey(const ValueKey('add-channel-member-alice')),
        );
        await tester.pump();
        await tester.tap(find.byKey(const ValueKey('add-channel-member-bob')));
        await tester.pump();
        expect(
          find.byKey(const ValueKey('add-channel-member-selected-alice')),
          findsOneWidget,
        );
        expect(
          find.byKey(const ValueKey('add-channel-member-selected-bob')),
          findsOneWidget,
        );
        await tester.tap(
          find.byKey(const ValueKey('add-channel-members-submit')),
        );
        await tester.pumpAndSettle();

        expect(attempts, 1);
        expect(
          find.byKey(const ValueKey('add-channel-member-selected-alice')),
          findsNothing,
        );
        expect(
          find.byKey(const ValueKey('add-channel-member-selected-bob')),
          findsOneWidget,
        );
        expect(
          find.byKey(const ValueKey('add-channel-member-alice')),
          findsNothing,
        );
        // The successful add must not remain selected for a retry.
        expect(
          tester
              .widgetList<InputChip>(find.byType(InputChip))
              .map((chip) => chip.key)
              .toList(),
          [const ValueKey('add-channel-member-selected-bob')],
        );
        await tester.tap(
          find.byKey(const ValueKey('add-channel-members-submit')),
        );
        await tester.pumpAndSettle();
        expect(attempts, 2);
        expect(submittedPubkeys, [
          ['alice', 'bob'],
          ['bob'],
        ]);
      },
    );

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

      await tester.tap(
        find.byKey(const ValueKey('channel-header-settings-trigger')),
      );
      await tester.pumpAndSettle();
      await tester.drag(
        find.byKey(const ValueKey('channel-details-page-list')),
        const Offset(0, -300),
      );
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

    testWidgets('Leave lives on the detail page instead of Manage', (
      tester,
    ) async {
      await tester.pumpWidget(
        _buildTestable(
          messages: const [],
          createChannelActions: (ref) => _FakeChannelActions(ref),
        ),
      );
      await tester.pumpAndSettle();

      await tester.tap(
        find.byKey(const ValueKey('channel-header-settings-trigger')),
      );
      await tester.pumpAndSettle();
      await tester.drag(
        find.byKey(const ValueKey('channel-details-page-list')),
        const Offset(0, -300),
      );
      await tester.pumpAndSettle();
      await tester.tap(find.text('Leave channel').last);
      await tester.pumpAndSettle();
      await tester.tap(find.text('Leave'));
      await tester.pumpAndSettle();

      expect(
        find.byKey(const ValueKey('channel-header-settings-trigger')),
        findsNothing,
      );
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
          members: [
            ChannelMember(
              pubkey: 'self',
              role: 'owner',
              joinedAt: DateTime(2025),
            ),
          ],
          canvasContent: List.generate(
            80,
            (index) => 'Canvas line $index',
          ).join('\n'),
        ),
      );
      await tester.pumpAndSettle();

      await tester.tap(
        find.byKey(const ValueKey('channel-header-settings-trigger')),
      );
      await tester.pumpAndSettle();
      await tester.tap(
        find.byKey(const ValueKey('channel-details-edit-action')),
      );
      await tester.pumpAndSettle();

      final sheet = find.byType(BottomSheet).last;
      expect(find.byType(BottomSheet), findsOneWidget);
      expect(tester.getSize(sheet).height, lessThanOrEqualTo(720));

      final sheetTop = tester.getTopLeft(sheet).dy;
      await tester.dragFrom(
        Offset(tester.view.physicalSize.width / 2, sheetTop + 12),
        const Offset(0, 800),
      );
      await tester.pumpAndSettle();

      expect(find.text('Manage channel'), findsNothing);
    });

    testWidgets('Edit updates name and description without legacy fields', (
      tester,
    ) async {
      String? updatedName;
      String? updatedDescription;
      await tester.pumpWidget(
        _buildTestable(
          messages: const [],
          members: [
            ChannelMember(
              pubkey: 'self',
              role: 'owner',
              joinedAt: DateTime(2025),
            ),
          ],
          createChannelActions: (ref) => _FakeChannelActions(
            ref,
            onUpdateChannel: (_, name, description) async {
              updatedName = name;
              updatedDescription = description;
            },
          ),
        ),
      );
      await tester.pumpAndSettle();

      await tester.tap(
        find.byKey(const ValueKey('channel-header-settings-trigger')),
      );
      await tester.pumpAndSettle();
      await tester.tap(
        find.byKey(const ValueKey('channel-details-edit-action')),
      );
      await tester.pumpAndSettle();

      expect(find.text('Mute'), findsOneWidget);
      expect(find.text('Leave channel'), findsNothing);
      expect(find.text('Topic'), findsNothing);
      expect(find.text('Purpose'), findsNothing);
      expect(find.text('Canvas'), findsOneWidget);

      final nameField = tester.widget<TextField>(
        find.byKey(const ValueKey('manage-channel-name')),
      );
      final descriptionField = tester.widget<TextField>(
        find.byKey(const ValueKey('manage-channel-description')),
      );
      expect(nameField.decoration?.labelText, isNull);
      expect(nameField.decoration?.hintText, 'Channel name');
      expect(nameField.decoration?.border, InputBorder.none);
      expect(descriptionField.decoration?.labelText, isNull);
      expect(descriptionField.decoration?.hintText, 'Description');
      expect(descriptionField.decoration?.border, InputBorder.none);
      final nameOutline = tester.getRect(
        find.byKey(const ValueKey('manage-channel-name-outline')),
      );
      final descriptionOutline = tester.getRect(
        find.byKey(const ValueKey('manage-channel-description-outline')),
      );
      expect(descriptionOutline.top - nameOutline.bottom, Grid.xs);

      await tester.enterText(
        find.byKey(const ValueKey('manage-channel-name')),
        '  #renamed  ',
      );
      await tester.enterText(
        find.byKey(const ValueKey('manage-channel-description')),
        'A new description',
      );
      await tester.pump();
      await tester.tap(
        find.byKey(const ValueKey('manage-channel-save-details')),
      );
      await tester.pumpAndSettle();

      expect(updatedName, 'renamed');
      expect(updatedDescription, 'A new description');
      expect(find.text('renamed'), findsOneWidget);
      expect(find.text('A new description'), findsOneWidget);
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
      expect(
        aliceTimestamp.style?.fontSize,
        messageTimestampTextStyle.fontSize,
      );
      expect(aliceTimestamp.style?.fontWeight, FontWeight.w400);
      expect(
        aliceTimestamp.style?.fontSize,
        lessThan(aliceText.style!.fontSize!),
      );
      expect(
        find.descendant(
          of: find.byKey(const ValueKey('message-row-msg1')),
          matching: find.text('·'),
        ),
        findsNothing,
      );
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

    testWidgets('keeps animated message avatars static and transparent', (
      tester,
    ) async {
      const posterUrl = 'https://relay.example/media/alice-poster.png';
      const animationUrl = 'https://relay.example/media/alice-avatar.png';
      final profileUrl =
          '$posterUrl#buzz-anim=${Uri.encodeComponent(animationUrl)}';
      final mediaClient = http_testing.MockClient(
        (_) async => http.Response.bytes(_transparentPng, 200),
      );
      addTearDown(mediaClient.close);

      await tester.pumpWidget(
        _buildTestable(
          messages: [
            _textMsg(id: 'animated-avatar', pubkey: 'alice', content: 'Hello'),
          ],
          users: {
            'alice': UserProfile(
              pubkey: 'alice',
              displayName: 'Alice',
              avatarUrl: profileUrl,
            ),
          },
          mediaClient: mediaClient,
        ),
      );
      await tester.pumpAndSettle();

      expect(
        tester.widget<CircleAvatar>(find.byType(CircleAvatar)).backgroundColor,
        Colors.transparent,
      );
      expect(tester.widget<MediaImage>(find.byType(MediaImage)).url, posterUrl);
      expect(
        find.byKey(const ValueKey('progressive-animated-avatar-animation')),
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

      Navigator.of(
        tester.element(find.byKey(const ValueKey('reaction-popover-tray'))),
      ).pop();
      await tester.pumpAndSettle();
      expect(messageActionBackdropActive.value, isFalse);
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

      Navigator.of(
        tester.element(find.byKey(const ValueKey('reaction-popover-tray'))),
      ).pop();
      await tester.pumpAndSettle();
      expect(messageActionBackdropActive.value, isFalse);
    });

    testWidgets('reaction-only and full actions share the stronger backdrop', (
      tester,
    ) async {
      addTearDown(() => messageActionBackdropActive.value = false);
      await tester.pumpWidget(
        _buildTestable(
          messages: [
            _systemMsg(
              id: 'reaction-only-blur',
              payload: {
                'type': 'member_joined',
                'actor': 'alice',
                'target': 'alice',
              },
            ),
            _textMsg(
              id: 'full-action-blur',
              pubkey: 'alice',
              content: 'Full action target',
              createdAt: 1100,
            ),
          ],
          users: const {
            'alice': UserProfile(pubkey: 'alice', displayName: 'Alice'),
          },
        ),
      );
      await tester.pumpAndSettle();

      await tester.longPress(
        find.byKey(const ValueKey('system-message-row-reaction-only-blur')),
      );
      await tester.pumpAndSettle();
      expect(messageActionBackdropActive.value, isTrue);
      final reactionBackdrop = tester.widget<BackdropFilter>(
        find.byKey(const ValueKey('reaction-popover-backdrop-filter')),
      );
      final reactionTint = tester.widget<ColoredBox>(
        find.byKey(const ValueKey('reaction-popover-background-tint')),
      );

      Navigator.of(
        tester.element(find.byKey(const ValueKey('reaction-popover-tray'))),
      ).pop();
      await tester.pumpAndSettle();
      expect(messageActionBackdropActive.value, isFalse);

      await tester.longPress(
        find.byKey(const ValueKey('message-row-full-action-blur')),
      );
      await tester.pumpAndSettle();
      expect(messageActionBackdropActive.value, isTrue);
      final fullBackdrop = tester.widget<BackdropFilter>(
        find.byKey(const ValueKey('message-actions-backdrop-filter')),
      );
      final fullTint = tester.widget<ColoredBox>(
        find.byKey(const ValueKey('message-actions-background')),
      );

      expect(fullBackdrop.filter, same(reactionBackdrop.filter));
      expect(fullTint.color, reactionTint.color);

      Navigator.of(
        tester.element(find.byKey(const ValueKey('message-action-surface'))),
      ).pop();
      await tester.pumpAndSettle();
      expect(messageActionBackdropActive.value, isFalse);
    });

    testWidgets('reaction-only popovers serialize concurrent requests', (
      tester,
    ) async {
      await tester.pumpWidget(
        _buildTestable(
          messages: const [],
          home: Scaffold(
            body: Consumer(
              builder: (context, ref, _) => TextButton(
                key: const ValueKey('concurrent-reaction-launcher'),
                onPressed: () {
                  final message = formatTimeline([
                    _systemMsg(
                      id: 'concurrent-reaction',
                      payload: {
                        'type': 'member_joined',
                        'actor': 'alice',
                        'target': 'alice',
                      },
                    ),
                  ]).single;
                  const anchorRect = Rect.fromLTWH(32, 260, 300, 72);
                  showMessageActions(
                    context: context,
                    ref: ref,
                    message: message,
                    channelId: _channelId,
                    canManageMessage: false,
                    anchorRect: anchorRect,
                  );
                  showMessageActions(
                    context: context,
                    ref: ref,
                    message: message,
                    channelId: _channelId,
                    canManageMessage: false,
                    anchorRect: anchorRect,
                  );
                },
                child: const Text('Open concurrent reactions'),
              ),
            ),
          ),
        ),
      );
      await tester.pumpAndSettle();

      await tester.tap(
        find.byKey(const ValueKey('concurrent-reaction-launcher')),
      );
      await tester.pumpAndSettle();

      expect(
        find.byKey(const ValueKey('reaction-popover-tray')),
        findsOneWidget,
      );
      expect(messageActionBackdropActive.value, isTrue);

      Navigator.of(
        tester.element(find.byKey(const ValueKey('reaction-popover-tray'))),
      ).pop();
      await tester.pumpAndSettle();
      expect(messageActionBackdropActive.value, isFalse);

      await tester.tap(
        find.byKey(const ValueKey('concurrent-reaction-launcher')),
      );
      await tester.pumpAndSettle();
      expect(
        find.byKey(const ValueKey('reaction-popover-tray')),
        findsOneWidget,
        reason: 'The presentation latch must release after dismissal.',
      );

      Navigator.of(
        tester.element(find.byKey(const ValueKey('reaction-popover-tray'))),
      ).pop();
      await tester.pumpAndSettle();
      expect(messageActionBackdropActive.value, isFalse);
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

      Navigator.of(
        tester.element(find.byKey(const ValueKey('reaction-popover-tray'))),
      ).pop();
      await tester.pumpAndSettle();
      expect(messageActionBackdropActive.value, isFalse);
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

      Navigator.of(
        tester.element(find.byKey(const ValueKey('reaction-popover-tray'))),
      ).pop();
      await tester.pumpAndSettle();
      expect(messageActionBackdropActive.value, isFalse);
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

      expect(
        find.byKey(const ValueKey('message-action-reaction-tray')),
        findsOneWidget,
      );
      expect(
        find.byKey(const ValueKey('message-action-preview')),
        findsOneWidget,
      );
      expect(
        find.byKey(const ValueKey('message-action-surface')),
        findsOneWidget,
      );
      expect(find.byType(BottomSheet), findsNothing);
      expect(find.text('Copy text'), findsOneWidget);

      Navigator.of(
        tester.element(find.byKey(const ValueKey('message-action-surface'))),
      ).pop();
      await tester.pumpAndSettle();
      expect(messageActionBackdropActive.value, isFalse);
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
      expect(
        find.descendant(
          of: unreadButton,
          matching: find.byIcon(LucideIcons.chevronUp),
        ),
        findsOneWidget,
      );
      expect(tester.getSize(unreadButton), const Size.square(48));
      final unreadRect = tester.getRect(unreadButton);
      expect(
        unreadRect.top,
        frostedAppBarHeight(
              tester.element(unreadButton),
              titleContentHeight: tester
                  .widget<FrostedAppBar>(find.byType(FrostedAppBar).first)
                  .titleContentHeight,
            ) +
            Grid.xs,
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
      expect(find.text('Latest'), findsNothing);
      expect(find.byIcon(LucideIcons.arrowDown), findsOneWidget);
      expect(find.byTooltip('Jump to latest message'), findsOneWidget);
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
        for (var frame = 0; frame < 15; frame += 1) {
          await tester.pump(const Duration(milliseconds: 16));
          expect(
            find.byKey(const ValueKey('channel-jump-to-latest')),
            findsNothing,
            reason:
                'Composer expansion must not expose Latest while tail-follow '
                'layout catches up.',
          );
        }
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

        for (final inset in const [80.0, 160.0, 240.0, 300.0]) {
          tester.view.viewInsets = FakeViewPadding(bottom: inset);
          await tester.pump(const Duration(milliseconds: 16));
          expect(
            find.byKey(const ValueKey('channel-jump-to-latest')),
            findsNothing,
            reason:
                'IME inset frames must not expose Latest while the followed '
                'tail is being realigned.',
          );
        }
        await tester.pump(androidImeMetricsSettleDelay);
        expect(
          find.byKey(const ValueKey('channel-jump-to-latest')),
          findsNothing,
          reason:
              'Latest must stay hidden when settled IME padding is applied.',
        );
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

    testWidgets(
      'seeds an already-visible Android keyboard into the channel tail layout',
      (tester) async {
        final previousPlatform = debugDefaultTargetPlatformOverride;
        debugDefaultTargetPlatformOverride = TargetPlatform.android;
        try {
          tester.view.physicalSize = const Size(400, 800);
          tester.view.devicePixelRatio = 1;
          tester.view.viewPadding = const FakeViewPadding(bottom: 24);
          tester.view.viewInsets = const FakeViewPadding(bottom: 300);
          addTearDown(tester.view.reset);

          final messages = [
            for (var i = 0; i < 20; i++)
              _textMsg(
                id: 'msg$i',
                pubkey: 'alice',
                content: i == 19
                    ? List.filled(8, 'Tall latest message').join('\n')
                    : 'Message $i',
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
            const ValueKey('channel-message-group-msg19'),
          );
          final composerDock = find.byKey(
            const ValueKey('channel-composer-dock'),
          );
          expect(latestMessage, findsOneWidget);
          expect(
            tester.getBottomLeft(latestMessage).dy,
            closeTo(tester.getTopLeft(composerDock).dy, 1),
          );
          expect(
            find.byKey(const ValueKey('channel-jump-to-latest')),
            findsNothing,
          );
        } finally {
          debugDefaultTargetPlatformOverride = previousPlatform;
        }
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
      final latestSurfaceFinder = find.byKey(
        const ValueKey('channel-jump-to-latest-surface'),
      );
      final latestSurface = tester.widget<Container>(latestSurfaceFinder);
      final latestDecoration = latestSurface.decoration! as BoxDecoration;
      expect(latestDecoration.shape, BoxShape.circle);
      expect(
        latestDecoration.color,
        AppTheme.light().colorScheme.surface.withValues(alpha: 0.72),
      );
      expect(
        (latestDecoration.border! as Border).top.color,
        AppTheme.light().colorScheme.onSurface.withValues(alpha: 0.08),
      );
      expect(
        tester.getSize(find.byKey(const ValueKey('channel-jump-to-latest'))),
        const Size.square(Grid.xl),
      );
      expect(
        tester
            .getCenter(find.byKey(const ValueKey('channel-jump-to-latest')))
            .dx,
        closeTo(tester.getCenter(messageList).dx, 0.1),
      );
      expect(
        tester
                .getTopLeft(find.byKey(const ValueKey('channel-composer-dock')))
                .dy -
            tester
                .getBottomRight(
                  find.byKey(const ValueKey('channel-jump-to-latest')),
                )
                .dy,
        closeTo(Grid.xs, 0.1),
      );
      final latestSwitcher = tester.widget<AnimatedSwitcher>(
        find.byKey(const ValueKey('channel-jump-to-latest-switcher')),
      );
      expect(latestSwitcher.duration, const Duration(milliseconds: 180));
      expect(latestSwitcher.reverseDuration, const Duration(milliseconds: 160));
      expect(latestSwitcher.switchInCurve, Curves.easeOutCubic);
      expect(latestSwitcher.switchOutCurve, Curves.easeInCubic);
      final latestScaleTransition = tester.widget<ScaleTransition>(
        find.descendant(
          of: find.byKey(const ValueKey('channel-jump-to-latest-switcher')),
          matching: find.byType(ScaleTransition),
        ),
      );
      expect(latestScaleTransition.alignment, Alignment.bottomCenter);
      final visualAnchor = tester.widget<Align>(
        find.byKey(const ValueKey('channel-jump-to-latest-visual-anchor')),
      );
      expect(visualAnchor.alignment, Alignment.bottomCenter);
      expect(tester.getSize(latestSurfaceFinder), const Size.square(Grid.lg));
      expect(
        tester.getBottomRight(latestSurfaceFinder).dy,
        closeTo(
          tester
              .getBottomRight(
                find.byKey(const ValueKey('channel-jump-to-latest')),
              )
              .dy,
          0.1,
        ),
      );
      expect(find.text('Latest'), findsNothing);
      expect(find.byIcon(LucideIcons.arrowDown), findsOneWidget);
      for (final container in tester.widgetList<Container>(
        find.descendant(
          of: find.byKey(const ValueKey('channel-jump-to-latest')),
          matching: find.byType(Container),
        ),
      )) {
        if (container.decoration case final BoxDecoration decoration) {
          expect(decoration.boxShadow, anyOf(isNull, isEmpty));
        }
      }
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
      await tester.pump();

      ScaleTransition exitingScaleTransition() {
        return tester.widget<ScaleTransition>(
          find.ancestor(
            of: find.byKey(const ValueKey('channel-jump-to-latest')),
            matching: find.byType(ScaleTransition),
          ),
        );
      }

      for (var frame = 0; frame < 60; frame += 1) {
        await tester.pump(const Duration(milliseconds: 16));
        if (exitingScaleTransition().scale.status == AnimationStatus.reverse) {
          break;
        }
      }
      expect(exitingScaleTransition().scale.status, AnimationStatus.reverse);
      await tester.pump(const Duration(milliseconds: 120));

      final collapsedScaleTransition = exitingScaleTransition();
      expect(collapsedScaleTransition.alignment, Alignment.bottomCenter);
      expect(collapsedScaleTransition.scale.value, lessThan(0.1));

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
      'a same-second local send follows its inserted row when the tail ID stays unchanged',
      (tester) async {
        tester.view.physicalSize = const Size(400, 600);
        tester.view.devicePixelRatio = 1;
        addTearDown(tester.view.reset);

        final initialMessages = [
          for (var i = 0; i < 39; i++)
            _textMsg(
              id: 'msg$i',
              pubkey: 'alice',
              content: 'Message $i',
              createdAt: 1000 + i,
            ),
          _textMsg(
            id: 'z-final',
            pubkey: 'alice',
            content: List.filled(20, 'Tall final message').join('\n'),
            createdAt: 2000,
          ),
        ];
        final messagesNotifier = _FakeMessagesNotifier(initialMessages);

        await tester.pumpWidget(
          _buildTestable(
            messages: const [],
            messagesNotifier: messagesNotifier,
            users: const {
              'alice': UserProfile(pubkey: 'alice', displayName: 'Alice'),
              'self': UserProfile(pubkey: 'self', displayName: 'Self'),
            },
          ),
        );
        await tester.pumpAndSettle();

        final messageList = find.byKey(const ValueKey('channel-message-list'));
        await tester.drag(messageList, const Offset(0, 300));
        await tester.pumpAndSettle();
        expect(findRichText('My same-second send'), findsNothing);

        final localSend = _textMsg(
          id: 'a-local',
          pubkey: 'self',
          content: 'My same-second send',
          createdAt: 2000,
        );
        final container = ProviderScope.containerOf(
          tester.element(find.byType(ChannelDetailPage)),
        );
        container
            .read(localMessageSendAnimationProvider(_channelId).notifier)
            .mark(localSend.id);
        messagesNotifier.setMessages([...initialMessages, localSend]);
        await tester.pumpAndSettle();

        expect(findRichText('My same-second send'), findsOneWidget);
        expect(
          find.byKey(const ValueKey('channel-jump-to-latest')),
          findsNothing,
        );
      },
    );

    testWidgets(
      'Latest reveals the channel tail while the Android keyboard stays open',
      (tester) async {
        final previousPlatform = debugDefaultTargetPlatformOverride;
        debugDefaultTargetPlatformOverride = TargetPlatform.android;
        try {
          tester.view.physicalSize = const Size(400, 800);
          tester.view.devicePixelRatio = 1;
          tester.view.viewPadding = const FakeViewPadding(bottom: 24);
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

          await tester.tap(find.text('Message #general'));
          await tester.pump();
          tester.view.viewInsets = const FakeViewPadding(bottom: 300);
          await tester.pump();
          await tester.pump(androidImeMetricsSettleDelay);
          await tester.pumpAndSettle();

          final textField = tester.widget<TextField>(find.byType(TextField));
          expect(textField.focusNode?.hasFocus, isTrue);

          final messageList = find.byKey(
            const ValueKey('channel-message-list'),
          );
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
          tester
              .widget<ScrollablePositionedList>(messageList)
              .itemScrollController!
              .jumpTo(index: 39);
          await tester.pumpAndSettle();

          expect(
            find.byKey(const ValueKey('channel-jump-to-latest')),
            findsOneWidget,
          );

          await tester.tap(
            find.byKey(const ValueKey('channel-jump-to-latest')),
          );
          await tester.pumpAndSettle();

          final latestMessage = find.byKey(
            const ValueKey('channel-message-group-msg39'),
          );
          final composerDock = find.byKey(
            const ValueKey('channel-composer-dock'),
          );
          expect(latestMessage, findsOneWidget);
          expect(textField.focusNode?.hasFocus, isTrue);
          expect(
            tester.getBottomLeft(latestMessage).dy,
            closeTo(tester.getTopLeft(composerDock).dy, 1),
          );
          expect(
            find.byKey(const ValueKey('channel-jump-to-latest')),
            findsNothing,
          );
        } finally {
          debugDefaultTargetPlatformOverride = previousPlatform;
        }
      },
    );

    testWidgets(
      'keeps the Latest gap stable above the Android composer and keyboard',
      (tester) async {
        final previousPlatform = debugDefaultTargetPlatformOverride;
        debugDefaultTargetPlatformOverride = TargetPlatform.android;
        try {
          tester.view.physicalSize = const Size(400, 800);
          tester.view.devicePixelRatio = 1;
          tester.view.viewPadding = const FakeViewPadding(bottom: 24);
          addTearDown(tester.view.reset);

          final initialMessages = [
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
              messages: initialMessages,
              users: const {
                'alice': UserProfile(pubkey: 'alice', displayName: 'Alice'),
              },
            ),
          );
          await tester.pumpAndSettle();

          final messageList = find.byKey(
            const ValueKey('channel-message-list'),
          );
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
          tester
              .widget<ScrollablePositionedList>(messageList)
              .itemScrollController!
              .jumpTo(index: 39);
          await tester.pumpAndSettle();

          final latestSurface = find.byKey(
            const ValueKey('channel-jump-to-latest-surface'),
          );
          final composerDock = find.byKey(
            const ValueKey('channel-composer-dock'),
          );
          double latestGap() =>
              tester.getTopLeft(composerDock).dy -
              tester.getBottomLeft(latestSurface).dy;

          final collapsedGap = latestGap();
          expect(collapsedGap, closeTo(Grid.xs, 0.5));

          await tester.tap(find.text('Message #general'));
          await tester.pump();
          await tester.pump();
          tester.view.viewInsets = const FakeViewPadding(bottom: 300);
          await tester.pump();
          await tester.pump(androidImeMetricsSettleDelay);
          await tester.pumpAndSettle();

          expect(find.byType(TextField), findsOneWidget);
          expect(latestGap(), closeTo(collapsedGap, 0.5));
        } finally {
          debugDefaultTargetPlatformOverride = previousPlatform;
        }
      },
    );

    testWidgets(
      'pins the current day below the app bar after its divider scrolls away',
      (tester) async {
        tester.view.physicalSize = const Size(400, 600);
        tester.view.devicePixelRatio = 1;
        addTearDown(tester.view.resetPhysicalSize);
        addTearDown(tester.view.resetDevicePixelRatio);

        final firstDay =
            DateTime(2025, 1, 1, 12).toUtc().millisecondsSinceEpoch ~/ 1000;
        final messages = [
          for (var day = 0; day < 3; day += 1)
            for (var index = 0; index < 10; index += 1)
              _textMsg(
                id: 'day-$day-message-$index',
                pubkey: 'alice',
                content: 'Day $day message $index',
                createdAt: firstDay + day * 86400 + index,
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
        final list = tester.widget<ScrollablePositionedList>(messageList);
        list.itemScrollController!.jumpTo(index: 14, alignment: 0.8);
        await tester.pumpAndSettle();

        final stickyHeader = find.byKey(
          const ValueKey('channel-sticky-date-header'),
        );
        final stickySurface = find.byKey(
          const ValueKey('channel-sticky-date-header-surface'),
        );
        expect(stickyHeader, findsOneWidget);
        expect(stickySurface, findsOneWidget);
        expect(
          find.descendant(
            of: stickyHeader,
            matching: find.text(formatDayHeading(firstDay + 86400)),
          ),
          findsOneWidget,
        );
        expect(
          tester.getTopLeft(stickySurface).dy,
          closeTo(
            frostedAppBarHeight(
                  tester.element(stickyHeader),
                  titleContentHeight: tester
                      .widget<FrostedAppBar>(find.byType(FrostedAppBar).first)
                      .titleContentHeight,
                ) +
                Grid.twelve,
            1,
          ),
        );
        expect(
          find.descendant(
            of: stickyHeader,
            matching: find.byType(BackdropFilter),
          ),
          findsOneWidget,
        );
      },
    );

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
          findsNothing,
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

    testWidgets('top action discovers a Huddle outside the timeline window', (
      tester,
    ) async {
      final now = DateTime.now().millisecondsSinceEpoch ~/ 1000;
      await tester.pumpWidget(
        _buildTestable(
          messages: const [],
          huddleLifecycle: [
            _huddleMsg(
              id: 'off-window-huddle',
              kind: EventKind.huddleStarted,
              createdAt: now,
            ),
          ],
        ),
      );
      await tester.pumpAndSettle();

      expect(find.byTooltip('Open Huddle'), findsOneWidget);
      expect(find.text('Huddle in progress'), findsNothing);
    });

    testWidgets('offers Join for a recent desktop-started huddle', (
      tester,
    ) async {
      final now = DateTime.now().millisecondsSinceEpoch ~/ 1000;
      await tester.pumpWidget(
        _buildTestable(
          messages: [
            _huddleMsg(
              id: 'active-huddle',
              kind: EventKind.huddleStarted,
              createdAt: now,
            ),
          ],
          users: {
            'alice': const UserProfile(pubkey: 'alice', displayName: 'Alice'),
          },
        ),
      );
      await tester.pumpAndSettle();

      expect(find.text('Huddle in progress'), findsOneWidget);
      final join = tester.widget<FilledButton>(
        find.widgetWithText(FilledButton, 'Join'),
      );
      expect(join.onPressed, isNotNull);
    });

    testWidgets('disables a different Huddle card during an active call', (
      tester,
    ) async {
      const otherHuddleChannelId = 'other-huddle-channel';
      final now = DateTime.now().millisecondsSinceEpoch ~/ 1000;
      await tester.pumpWidget(
        _buildTestable(
          messages: [
            _huddleMsg(
              id: 'current-huddle',
              kind: EventKind.huddleStarted,
              pubkey: 'self',
              createdAt: now,
            ),
            _huddleMsg(
              id: 'other-huddle',
              kind: EventKind.huddleStarted,
              pubkey: 'alice',
              createdAt: now,
              ephemeralChannelId: otherHuddleChannelId,
            ),
          ],
          users: const {
            'alice': UserProfile(pubkey: 'alice', displayName: 'Alice'),
            'self': UserProfile(pubkey: 'self', displayName: 'Self'),
          },
          relayConfigNotifier: _HuddleRelayConfigNotifier(),
          huddleCurrentPubkey: 'self',
          huddleMediaFactory: _HuddleTestMedia.new,
          huddleTransportFactory: (_) => _HuddleTestTransport(),
        ),
      );
      await tester.pumpAndSettle();

      await tester.tap(
        find.byKey(const ValueKey('huddle-Join-$_huddleChannelId')),
      );
      await tester.pumpAndSettle();
      await tester.tap(find.byKey(const ValueKey('huddle-minimize')));
      await tester.pumpAndSettle();

      final otherJoin = tester.widget<FilledButton>(
        find.byKey(const ValueKey('huddle-Join-$otherHuddleChannelId')),
      );
      expect(otherJoin.onPressed, isNull);
    });

    testWidgets('marks an expired Huddle card as ended', (tester) async {
      final now = DateTime.now().millisecondsSinceEpoch ~/ 1000;
      await tester.pumpWidget(
        _buildTestable(
          messages: [
            _huddleMsg(
              id: 'expired-huddle',
              kind: EventKind.huddleStarted,
              createdAt: now - const Duration(hours: 2).inSeconds,
            ),
          ],
          users: {
            'alice': const UserProfile(pubkey: 'alice', displayName: 'Alice'),
          },
        ),
      );
      await tester.pumpAndSettle();

      expect(find.text('Huddle ended'), findsOneWidget);
      expect(find.widgetWithText(FilledButton, 'Join'), findsNothing);
      final ended = tester.widget<FilledButton>(
        find.widgetWithText(FilledButton, 'Ended'),
      );
      expect(ended.onPressed, isNull);
    });

    testWidgets(
      'offers a new Huddle when a stale invitation is no longer joinable',
      (tester) async {
        final now = DateTime.now().millisecondsSinceEpoch ~/ 1000;
        final relaySession = _ReconnectingRelaySession();
        var transportCount = 0;

        await tester.pumpWidget(
          _buildTestable(
            messages: [
              _huddleMsg(
                id: 'stale-huddle',
                kind: EventKind.huddleStarted,
                pubkey: 'desktop',
                createdAt: now,
              ),
            ],
            users: const {
              'desktop': UserProfile(pubkey: 'desktop'),
              'self': UserProfile(pubkey: 'self'),
            },
            relayConfigNotifier: _HuddleRelayConfigNotifier(),
            relaySessionNotifier: relaySession,
            huddleCurrentPubkey: 'self',
            huddleMediaFactory: _HuddleTestMedia.new,
            huddleTransportFactory: (_) {
              transportCount++;
              return _HuddleTestTransport(
                connectError: transportCount == 1
                    ? const HuddleTransportError(
                        code: HuddleTransportErrorCode.relayRejected,
                        message: 'not a member',
                      )
                    : null,
              );
            },
          ),
        );
        await tester.pumpAndSettle();

        await tester.tap(find.widgetWithText(FilledButton, 'Join'));
        await tester.pumpAndSettle();

        expect(
          find.text('This Huddle is no longer available.'),
          findsOneWidget,
        );
        expect(find.byTooltip('Start a new Huddle'), findsOneWidget);
        expect(
          find.descendant(
            of: find.byKey(const ValueKey('huddle-retry')),
            matching: find.byIcon(LucideIcons.refreshCw),
          ),
          findsNothing,
        );

        await tester.tap(find.byKey(const ValueKey('huddle-retry')));
        await tester.pumpAndSettle();

        expect(relaySession.publishedKinds.take(2), [
          9007,
          EventKind.huddleStarted,
        ]);
        expect(
          find.byKey(const ValueKey('huddle-mute-toggle')),
          findsOneWidget,
        );
        expect(find.byKey(const ValueKey('huddle-retry')), findsNothing);
      },
    );

    testWidgets(
      'offers a Settings recovery path instead of a blind retry after a '
      'microphone denial',
      (tester) async {
        final now = DateTime.now().millisecondsSinceEpoch ~/ 1000;
        final media = _HuddleTestMedia(
          permission: HuddleMicrophonePermission.denied,
        );
        await tester.pumpWidget(
          _buildTestable(
            messages: [
              _huddleMsg(
                id: 'mic-denied-huddle',
                kind: EventKind.huddleStarted,
                pubkey: 'desktop',
                createdAt: now,
              ),
            ],
            users: const {
              'desktop': UserProfile(pubkey: 'desktop'),
              'self': UserProfile(pubkey: 'self'),
            },
            relayConfigNotifier: _HuddleRelayConfigNotifier(),
            huddleCurrentPubkey: 'self',
            huddleMediaFactory: () => media,
            huddleTransportFactory: (_) => _HuddleTestTransport(),
          ),
        );
        await tester.pumpAndSettle();

        await tester.tap(find.widgetWithText(FilledButton, 'Join'));
        await tester.pumpAndSettle();

        // A denied microphone must NOT surface the generic "Try again" that
        // deterministically fails again — only the Settings recovery path.
        expect(find.byTooltip('Try again'), findsNothing);
        expect(find.byTooltip('Open Settings'), findsOneWidget);
        expect(
          find.descendant(
            of: find.byKey(const ValueKey('huddle-retry')),
            matching: find.byIcon(LucideIcons.settings),
          ),
          findsOneWidget,
        );

        await tester.tap(find.byKey(const ValueKey('huddle-retry')));
        await tester.pumpAndSettle();

        expect(media.openSettingsCalls, 1);
      },
    );

    testWidgets('does not leave a stale Huddle card in a retry loop', (
      tester,
    ) async {
      final now = DateTime.now().millisecondsSinceEpoch ~/ 1000;
      await tester.pumpWidget(
        _buildTestable(
          messages: [
            _huddleMsg(
              id: 'stale-huddle-card',
              kind: EventKind.huddleStarted,
              pubkey: 'desktop',
              createdAt: now,
            ),
          ],
          users: const {
            'desktop': UserProfile(pubkey: 'desktop'),
            'self': UserProfile(pubkey: 'self'),
          },
          relayConfigNotifier: _HuddleRelayConfigNotifier(),
          huddleCurrentPubkey: 'self',
          huddleMediaFactory: _HuddleTestMedia.new,
          huddleTransportFactory: (_) => _HuddleTestTransport(
            connectError: const HuddleTransportError(
              code: HuddleTransportErrorCode.relayRejected,
              message: 'not a member',
            ),
          ),
        ),
      );
      await tester.pumpAndSettle();

      await tester.tap(find.widgetWithText(FilledButton, 'Join'));
      await tester.pumpAndSettle();
      await tester.tap(find.byKey(const ValueKey('huddle-minimize')));
      await tester.pumpAndSettle();

      expect(find.widgetWithText(FilledButton, 'Retry'), findsNothing);
      expect(find.widgetWithText(FilledButton, 'Start new'), findsOneWidget);
      expect(find.byTooltip('Start Huddle'), findsOneWidget);
      expect(
        find.descendant(
          of: find.byKey(const ValueKey('channel-huddle-button')),
          matching: find.byIcon(LucideIcons.headphones),
        ),
        findsOneWidget,
      );
      expect(find.byIcon(LucideIcons.headphoneOff), findsNothing);
    });

    testWidgets('failed admission cannot publish Huddle leave lifecycle', (
      tester,
    ) async {
      final now = DateTime.now().millisecondsSinceEpoch ~/ 1000;
      final relaySession = _ReconnectingRelaySession();
      await tester.pumpWidget(
        _buildTestable(
          messages: [
            _huddleMsg(
              id: 'unavailable-huddle',
              kind: EventKind.huddleStarted,
              pubkey: 'desktop',
              createdAt: now,
            ),
          ],
          users: const {
            'desktop': UserProfile(pubkey: 'desktop'),
            'self': UserProfile(pubkey: 'self'),
          },
          relayConfigNotifier: _HuddleRelayConfigNotifier(),
          relaySessionNotifier: relaySession,
          huddleCurrentPubkey: 'self',
          huddleMediaFactory: _HuddleTestMedia.new,
          huddleTransportFactory: (_) => _HuddleTestTransport(
            connectError: const HuddleTransportError(
              code: HuddleTransportErrorCode.relayRejected,
              message: 'not a member',
            ),
          ),
        ),
      );
      await tester.pumpAndSettle();

      await tester.tap(find.widgetWithText(FilledButton, 'Join'));
      await tester.pumpAndSettle();

      await tester.tap(find.byKey(const ValueKey('huddle-leave')));
      await tester.pumpAndSettle();

      expect(relaySession.publishedKinds, isEmpty);
    });

    testWidgets('shows the flapping bee instead of an avatar while joining', (
      tester,
    ) async {
      final now = DateTime.now().millisecondsSinceEpoch ~/ 1000;
      final connectGate = Completer<void>();
      final transport = _HuddleTestTransport(connectGate: connectGate.future);

      await tester.pumpWidget(
        _buildTestable(
          messages: [
            _huddleMsg(
              id: 'loading-call-layout',
              kind: EventKind.huddleStarted,
              pubkey: 'desktop',
              createdAt: now,
            ),
          ],
          users: const {
            'desktop': UserProfile(pubkey: 'desktop'),
            'self': UserProfile(pubkey: 'self'),
          },
          relayConfigNotifier: _HuddleRelayConfigNotifier(),
          huddleCurrentPubkey: 'self',
          huddleMediaFactory: _HuddleTestMedia.new,
          huddleTransportFactory: (_) => transport,
        ),
      );
      await tester.pumpAndSettle();

      await tester.tap(find.widgetWithText(FilledButton, 'Join'));
      await tester.pump();
      await tester.pump();

      final loadingBee = find.byKey(const ValueKey('huddle-loading-bee'));
      expect(loadingBee, findsOneWidget);
      expect(find.byType(FlappingBee), findsOneWidget);
      expect(tester.widget<FlappingBee>(loadingBee).width, 60);
      expect(find.bySemanticsLabel('Joining Huddle'), findsOneWidget);
      expect(
        find.byKey(const ValueKey('huddle-participant-avatar-self')),
        findsNothing,
      );
      final initialFlap = tester.widget<FlappingBee>(loadingBee).flapAmount;
      await tester.pump(const Duration(milliseconds: 120));
      expect(
        tester.widget<FlappingBee>(loadingBee).flapAmount,
        isNot(initialFlap),
      );

      connectGate.complete();
      await tester.pumpAndSettle();

      expect(loadingBee, findsNothing);
      expect(
        find.byKey(const ValueKey('huddle-participant-avatar-self')),
        findsOneWidget,
      );
    });

    testWidgets('shows response dots only until the agent starts speaking', (
      tester,
    ) async {
      final now = DateTime.now().millisecondsSinceEpoch ~/ 1000;
      final typing = _FakeTypingNotifier([
        TypingEntry(
          pubkey: 'agent',
          expiresAtMs: DateTime.now().millisecondsSinceEpoch + 8000,
        ),
      ], channelId: _huddleChannelId);
      final transport = _HuddleTestTransport(
        peers: const {
          1: HuddlePeer(pubkey: 'self', peerIndex: 1, epoch: 0),
          2: HuddlePeer(pubkey: 'agent', peerIndex: 2, epoch: 0),
        },
      );

      await tester.pumpWidget(
        _buildTestable(
          messages: [
            _huddleMsg(
              id: 'working-agent-call',
              kind: EventKind.huddleStarted,
              pubkey: 'self',
              createdAt: now,
            ),
          ],
          users: const {
            'agent': UserProfile(pubkey: 'agent', displayName: 'Pollen'),
            'self': UserProfile(pubkey: 'self', displayName: 'Self'),
          },
          members: [
            ChannelMember(
              pubkey: 'agent',
              role: 'bot',
              joinedAt: DateTime(2025),
            ),
          ],
          loadChannelBotPubkeys: () async => const {'agent'},
          huddleTypingNotifier: typing,
          relayConfigNotifier: _HuddleRelayConfigNotifier(),
          huddleCurrentPubkey: 'self',
          huddleMediaFactory: _HuddleTestMedia.new,
          huddleTransportFactory: (_) => transport,
        ),
      );
      await tester.pumpAndSettle();

      await tester.tap(find.widgetWithText(FilledButton, 'Join'));
      await tester.pump();
      await tester.pump(const Duration(milliseconds: 500));
      await tester.pump(const Duration(milliseconds: 200));

      expect(
        find.byKey(const ValueKey('huddle-agent-preparing-response-agent')),
        findsOneWidget,
      );
      expect(
        find.byKey(const ValueKey('huddle-avatar-image-agent')),
        findsNothing,
      );
      expect(
        find.bySemanticsLabel('Pollen, preparing a response'),
        findsOneWidget,
      );
      // The preparing transition must be announced live. The outer avatar node
      // excludes descendant semantics, so it must itself become a live region.
      expect(
        tester
            .getSemantics(
              find.byKey(const ValueKey('huddle-participant-avatar-agent')),
            )
            .flagsCollection
            .isLiveRegion,
        isTrue,
      );
      final dotFinder = find.byKey(const ValueKey('bouncing-dot-2'));
      final initialDotOffset = tester
          .widget<Transform>(dotFinder)
          .transform
          .getTranslation()
          .y;
      await tester.pump(const Duration(milliseconds: 120));
      expect(
        tester.widget<Transform>(dotFinder).transform.getTranslation().y,
        isNot(initialDotOffset),
      );

      transport.emitRemoteAudio(peerIndex: 2);
      await tester.pump();
      await tester.pump(const Duration(milliseconds: 200));

      expect(
        find.byKey(const ValueKey('huddle-agent-preparing-response-agent')),
        findsNothing,
      );
      expect(
        find.byKey(const ValueKey('huddle-avatar-image-agent')),
        findsOneWidget,
      );
      expect(find.bySemanticsLabel('Pollen, speaking'), findsOneWidget);

      // A brief audio gap must not revive the waiting state while the same
      // working signal is still active.
      await tester.pump(const Duration(milliseconds: 650));
      expect(
        find.byKey(const ValueKey('huddle-agent-preparing-response-agent')),
        findsNothing,
      );
      expect(
        find.byKey(const ValueKey('huddle-avatar-image-agent')),
        findsOneWidget,
      );

      typing.setEntries(const []);
      await tester.pump();
      await tester.pump(const Duration(milliseconds: 200));

      expect(
        find.byKey(const ValueKey('huddle-agent-preparing-response-agent')),
        findsNothing,
      );
      expect(
        find.byKey(const ValueKey('huddle-avatar-image-agent')),
        findsOneWidget,
      );
    });

    testWidgets(
      'shows preparing indicator for a Huddle-only bot without a parent role',
      (tester) async {
        // Regression: a valid ephemeral Huddle bot with no parent bot role and
        // no directory identity must still enter the preparing state from its
        // Huddle typing. Classification must derive from authoritative
        // ephemeral Huddle bot membership, not parent-channel classification.
        final now = DateTime.now().millisecondsSinceEpoch ~/ 1000;
        final typing = _FakeTypingNotifier([
          TypingEntry(
            pubkey: 'agent',
            expiresAtMs: DateTime.now().millisecondsSinceEpoch + 8000,
          ),
        ], channelId: _huddleChannelId);
        final transport = _HuddleTestTransport(
          peers: const {
            1: HuddlePeer(pubkey: 'self', peerIndex: 1, epoch: 0),
            2: HuddlePeer(pubkey: 'agent', peerIndex: 2, epoch: 0),
          },
        );

        await tester.pumpWidget(
          _buildTestable(
            messages: [
              _huddleMsg(
                id: 'huddle-only-working-agent-call',
                kind: EventKind.huddleStarted,
                pubkey: 'self',
                createdAt: now,
              ),
            ],
            users: const {
              'agent': UserProfile(pubkey: 'agent', displayName: 'Pollen'),
              'self': UserProfile(pubkey: 'self', displayName: 'Self'),
            },
            // Bot membership is supplied ONLY through the ephemeral Huddle, and
            // deliberately not through the parent channel (`members`).
            huddleMembers: [
              ChannelMember(
                pubkey: 'agent',
                role: 'bot',
                joinedAt: DateTime(2025),
              ),
            ],
            huddleTypingNotifier: typing,
            relayConfigNotifier: _HuddleRelayConfigNotifier(),
            huddleCurrentPubkey: 'self',
            huddleMediaFactory: _HuddleTestMedia.new,
            huddleTransportFactory: (_) => transport,
          ),
        );
        await tester.pumpAndSettle();

        await tester.tap(find.widgetWithText(FilledButton, 'Join'));
        await tester.pump();
        await tester.pump(const Duration(milliseconds: 500));
        await tester.pump(const Duration(milliseconds: 200));

        expect(
          find.byKey(const ValueKey('huddle-agent-preparing-response-agent')),
          findsOneWidget,
        );
        expect(
          find.byKey(const ValueKey('huddle-avatar-image-agent')),
          findsNothing,
        );
        expect(
          find.bySemanticsLabel('Pollen, preparing a response'),
          findsOneWidget,
        );
      },
    );

    testWidgets('does not revive response dots when typing follows audio', (
      tester,
    ) async {
      final now = DateTime.now().millisecondsSinceEpoch ~/ 1000;
      final typing = _FakeTypingNotifier(const [], channelId: _huddleChannelId);
      final transport = _HuddleTestTransport(
        peers: const {
          1: HuddlePeer(pubkey: 'self', peerIndex: 1, epoch: 0),
          2: HuddlePeer(pubkey: 'agent', peerIndex: 2, epoch: 0),
        },
      );

      await tester.pumpWidget(
        _buildTestable(
          messages: [
            _huddleMsg(
              id: 'audio-before-working-signal',
              kind: EventKind.huddleStarted,
              pubkey: 'self',
              createdAt: now,
            ),
          ],
          users: const {
            'agent': UserProfile(pubkey: 'agent', displayName: 'Pollen'),
            'self': UserProfile(pubkey: 'self', displayName: 'Self'),
          },
          huddleTypingNotifier: typing,
          relayConfigNotifier: _HuddleRelayConfigNotifier(),
          huddleCurrentPubkey: 'self',
          huddleMediaFactory: _HuddleTestMedia.new,
          huddleTransportFactory: (_) => transport,
        ),
      );
      await tester.pumpAndSettle();
      await tester.tap(find.widgetWithText(FilledButton, 'Join'));
      await tester.pumpAndSettle();

      transport.emitRemoteAudio(peerIndex: 2);
      await tester.pump();
      expect(find.bySemanticsLabel('Pollen, speaking'), findsOneWidget);

      // Let the active-speaker window close before the independently
      // transported typing signal arrives.
      await tester.pump(const Duration(milliseconds: 650));
      typing.setEntries([
        TypingEntry(
          pubkey: 'agent',
          expiresAtMs: DateTime.now().millisecondsSinceEpoch + 8000,
        ),
      ]);
      await tester.pump();
      await tester.pump(const Duration(milliseconds: 200));

      expect(
        find.byKey(const ValueKey('huddle-agent-preparing-response-agent')),
        findsNothing,
      );
      expect(
        find.byKey(const ValueKey('huddle-avatar-image-agent')),
        findsOneWidget,
      );
    });

    testWidgets('revives response dots for a new turn after audio completes', (
      tester,
    ) async {
      // Regression: a completed working cycle followed by a genuinely new one
      // must show the preparing indicator again. A single audio-seen latch
      // conflates late same-turn typing (suppress) with a fresh turn (show).
      final now = DateTime.now().millisecondsSinceEpoch ~/ 1000;
      final typing = _FakeTypingNotifier(const [], channelId: _huddleChannelId);
      final transport = _HuddleTestTransport(
        peers: const {
          1: HuddlePeer(pubkey: 'self', peerIndex: 1, epoch: 0),
          2: HuddlePeer(pubkey: 'agent', peerIndex: 2, epoch: 0),
        },
      );

      await tester.pumpWidget(
        _buildTestable(
          messages: [
            _huddleMsg(
              id: 'new-turn-after-audio',
              kind: EventKind.huddleStarted,
              pubkey: 'self',
              createdAt: now,
            ),
          ],
          users: const {
            'agent': UserProfile(pubkey: 'agent', displayName: 'Pollen'),
            'self': UserProfile(pubkey: 'self', displayName: 'Self'),
          },
          huddleMembers: [
            ChannelMember(
              pubkey: 'agent',
              role: 'bot',
              joinedAt: DateTime(2025),
            ),
          ],
          huddleTypingNotifier: typing,
          relayConfigNotifier: _HuddleRelayConfigNotifier(),
          huddleCurrentPubkey: 'self',
          huddleMediaFactory: _HuddleTestMedia.new,
          huddleTransportFactory: (_) => transport,
        ),
      );
      await tester.pumpAndSettle();
      await tester.tap(find.widgetWithText(FilledButton, 'Join'));
      await tester.pumpAndSettle();

      // Turn 1: audio speaks, then late same-turn typing must stay suppressed.
      transport.emitRemoteAudio(peerIndex: 2);
      await tester.pump();
      await tester.pump(const Duration(milliseconds: 650));
      typing.setEntries([
        TypingEntry(
          pubkey: 'agent',
          expiresAtMs: DateTime.now().millisecondsSinceEpoch + 8000,
        ),
      ]);
      await tester.pump();
      await tester.pump(const Duration(milliseconds: 200));
      expect(
        find.byKey(const ValueKey('huddle-agent-preparing-response-agent')),
        findsNothing,
      );

      // Turn 1's working signal completes.
      typing.setEntries(const []);
      await tester.pump();
      await tester.pump(const Duration(milliseconds: 200));

      // Turn 2: a fresh working signal within the 1.2 s cooldown must show the
      // preparing indicator and announce it live again.
      typing.setEntries([
        TypingEntry(
          pubkey: 'agent',
          expiresAtMs: DateTime.now().millisecondsSinceEpoch + 8000,
        ),
      ]);
      await tester.pump();
      await tester.pump(const Duration(milliseconds: 200));

      expect(
        find.byKey(const ValueKey('huddle-agent-preparing-response-agent')),
        findsOneWidget,
      );
      expect(
        find.byKey(const ValueKey('huddle-avatar-image-agent')),
        findsNothing,
      );
      expect(
        find.bySemanticsLabel('Pollen, preparing a response'),
        findsOneWidget,
      );
      expect(
        tester
            .getSemantics(
              find.byKey(const ValueKey('huddle-participant-avatar-agent')),
            )
            .flagsCollection
            .isLiveRegion,
        isTrue,
      );
    });

    testWidgets(
      'does not restart the profile subscription on speaker-level updates',
      (tester) async {
        // Regression: the logical-participant provider must select only
        // roster-relevant session fields. Watching the whole session would
        // recompute at the 50 ms speaker-level flush cadence, tearing down and
        // recreating the kind-0 profile subscription ~20x/sec while anyone is
        // speaking.
        final now = DateTime.now().millisecondsSinceEpoch ~/ 1000;
        final relaySession = _ProfileSubscriptionRelaySession();
        final transport = _HuddleTestTransport(
          peers: const {
            1: HuddlePeer(pubkey: 'self', peerIndex: 1, epoch: 0),
            2: HuddlePeer(pubkey: 'agent', peerIndex: 2, epoch: 0),
          },
        );

        await tester.pumpWidget(
          _buildTestable(
            messages: [
              _huddleMsg(
                id: 'speaker-level-profile-churn',
                kind: EventKind.huddleStarted,
                pubkey: 'self',
                createdAt: now,
              ),
            ],
            users: const {
              'agent': UserProfile(pubkey: 'agent', displayName: 'Pollen'),
              'self': UserProfile(pubkey: 'self', displayName: 'Self'),
            },
            relaySessionNotifier: relaySession,
            relayConfigNotifier: _HuddleRelayConfigNotifier(),
            huddleCurrentPubkey: 'self',
            huddleMediaFactory: _HuddleTestMedia.new,
            huddleTransportFactory: (_) => transport,
          ),
        );
        await tester.pumpAndSettle();
        await tester.tap(find.widgetWithText(FilledButton, 'Join'));
        await tester.pumpAndSettle();

        final baseline = relaySession.profileSubscriptions;
        expect(baseline, greaterThan(0));

        // Drive continuous speaker-level flushes: each frame within the 600 ms
        // active window refreshes the level and schedules a 50 ms flush that
        // republishes the whole session state.
        for (var i = 0; i < 10; i++) {
          transport.emitRemoteAudio(peerIndex: 2, sequence: i + 1);
          await tester.pump(const Duration(milliseconds: 50));
        }
        await tester.pumpAndSettle();

        expect(relaySession.profileSubscriptions, baseline);
      },
    );

    testWidgets(
      'opens the sparse full-screen call with avatar and audio controls',
      (tester) async {
        final now = DateTime.now().millisecondsSinceEpoch ~/ 1000;
        final media = _HuddleTestMedia();
        final transport = _HuddleTestTransport(
          peers: const {
            1: HuddlePeer(pubkey: 'desktop', peerIndex: 1, epoch: 0),
            2: HuddlePeer(pubkey: 'self', peerIndex: 2, epoch: 0),
            3: HuddlePeer(pubkey: 'agent', peerIndex: 3, epoch: 0),
          },
        );
        final users = _FakeUserCacheNotifier(const {
          'desktop': UserProfile(pubkey: 'desktop', displayName: 'Miles'),
          'agent': UserProfile(pubkey: 'agent', displayName: 'Pollen'),
          'self': UserProfile(pubkey: 'self', displayName: 'Self'),
        });
        final navigator = _RecordingNavigatorObserver();
        String? leftChannelId;
        final hapticCalls = <MethodCall>[];
        tester.binding.defaultBinaryMessenger.setMockMethodCallHandler(
          SystemChannels.platform,
          (call) async {
            if (call.method == 'HapticFeedback.vibrate') {
              hapticCalls.add(call);
            }
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
              _huddleMsg(
                id: 'active-call-layout',
                kind: EventKind.huddleStarted,
                pubkey: 'self',
                createdAt: now,
              ),
            ],
            userCacheNotifier: users,
            huddleMembers: [
              ChannelMember(
                pubkey: 'agent',
                role: 'bot',
                joinedAt: DateTime(2025),
              ),
            ],
            navigatorObservers: [navigator],
            relayConfigNotifier: _HuddleRelayConfigNotifier(),
            huddleCurrentPubkey: 'self',
            huddleHumanCountLoader: (_) async => 2,
            huddleMediaFactory: () => media,
            huddleTransportFactory: (_) => transport,
            createChannelActions: (ref) => _FakeChannelActions(
              ref,
              onLeaveChannel: (channelId) async => leftChannelId = channelId,
            ),
          ),
        );
        await tester.pumpAndSettle();

        await tester.tap(find.widgetWithText(FilledButton, 'Join'));
        await tester.pumpAndSettle();

        expect(navigator.pushedRoutes.last, isA<PageRouteBuilder<void>>());
        final route = navigator.pushedRoutes.last as PageRouteBuilder<void>;
        expect(route.transitionDuration, const Duration(milliseconds: 280));
        expect(find.byKey(const ValueKey('huddle-minimize')), findsOneWidget);
        expect(find.byKey(const ValueKey('huddle-leave')), findsOneWidget);
        expect(
          tester.getSize(find.byKey(const ValueKey('huddle-leave'))),
          const Size.square(64),
        );
        expect(
          tester.getSize(find.byKey(const ValueKey('huddle-speaker-toggle'))),
          const Size.square(80),
        );
        expect(
          tester.getSize(find.byKey(const ValueKey('huddle-mute-toggle'))),
          const Size.square(80),
        );
        expect(
          tester.getSize(find.byKey(const ValueKey('huddle-emoji-reactions'))),
          const Size.square(80),
        );
        expect(
          (tester
                      .widget<Padding>(
                        find.byKey(const ValueKey('huddle-call-controls')),
                      )
                      .padding
                  as EdgeInsets)
              .bottom,
          0,
        );
        final speakerCenter = tester.getCenter(
          find.byKey(const ValueKey('huddle-speaker-toggle')),
        );
        final muteCenter = tester.getCenter(
          find.byKey(const ValueKey('huddle-mute-toggle')),
        );
        final emojiCenter = tester.getCenter(
          find.byKey(const ValueKey('huddle-emoji-reactions')),
        );
        expect(speakerCenter.dy, closeTo(muteCenter.dy, 0.01));
        expect(emojiCenter.dy, closeTo(muteCenter.dy, 0.01));
        expect(
          (speakerCenter.dx + emojiCenter.dx) / 2,
          closeTo(muteCenter.dx, 0.01),
        );
        expect(find.text('Miles'), findsNothing);
        expect(find.text('Pollen'), findsNothing);
        expect(find.text('You'), findsNothing);
        expect(
          find.byWidgetPredicate(
            (widget) => widget is Semantics && widget.properties.label == 'You',
          ),
          findsOneWidget,
        );
        expect(
          tester
              .widget<Align>(
                find.byKey(const ValueKey('huddle-remote-participant-group')),
              )
              .alignment,
          const Alignment(0, 0.35),
        );
        expect(
          tester
              .widget<Align>(
                find.byKey(const ValueKey('huddle-local-participant')),
              )
              .alignment,
          const Alignment(0, -0.35),
        );
        expect(
          tester
              .getCenter(
                find.byKey(const ValueKey('huddle-speaking-ring-desktop')),
              )
              .dy,
          lessThan(
            tester
                .getCenter(
                  find.byKey(const ValueKey('huddle-speaking-ring-self')),
                )
                .dy,
          ),
        );
        expect(
          tester.getSize(find.byType(CircleAvatar).first),
          const Size.square(104),
        );
        expect(find.byIcon(LucideIcons.userRound), findsNWidgets(3));
        await tester.tap(
          find.byKey(const ValueKey('huddle-participant-avatar-desktop')),
        );
        await tester.pump();
        expect(hapticCalls, hasLength(1));
        expect(hapticCalls.last.arguments, 'HapticFeedbackType.selectionClick');
        expect(
          find.byKey(const ValueKey('huddle-participant-modal-backdrop')),
          findsOneWidget,
        );
        expect(
          find.byKey(const ValueKey('huddle-participant-spotlight')),
          findsOneWidget,
        );
        await tester.pumpAndSettle();
        expect(find.text('Miles'), findsOneWidget);
        expect(
          find.byWidgetPredicate(
            (widget) =>
                widget is Semantics && widget.properties.label == 'Miles',
          ),
          findsNWidgets(2),
        );
        transport.emitRemoteAudio();
        await tester.pump();
        expect(
          find.byWidgetPredicate(
            (widget) =>
                widget is Semantics &&
                widget.properties.label == 'Miles, speaking',
          ),
          findsNWidgets(2),
        );
        users.replace(
          const UserProfile(pubkey: 'desktop', displayName: 'Miles Davis'),
        );
        await tester.pump();
        expect(find.text('Miles'), findsNothing);
        expect(find.text('Miles Davis'), findsOneWidget);
        expect(
          find.byWidgetPredicate(
            (widget) =>
                widget is Semantics &&
                widget.properties.label == 'Miles Davis, speaking',
          ),
          findsNWidgets(2),
        );
        users.replace(
          const UserProfile(pubkey: 'desktop', displayName: 'Miles'),
        );
        await tester.pump();
        expect(find.text('Pollen'), findsNothing);
        expect(find.byKey(const ValueKey('huddle-leave')), findsOneWidget);
        expect(
          find.byKey(const ValueKey('huddle-participant-label-desktop')),
          findsNothing,
        );

        transport.emitPeerLeave(1);
        await tester.pumpAndSettle();
        expect(
          find.byKey(const ValueKey('huddle-participant-spotlight')),
          findsNothing,
        );
        transport.emitPeerJoin(
          const HuddlePeer(pubkey: 'desktop', peerIndex: 1, epoch: 1),
        );
        await tester.pump();

        await tester.tap(
          find.byKey(const ValueKey('huddle-participant-avatar-agent')),
        );
        await tester.pumpAndSettle();
        expect(hapticCalls, hasLength(2));
        expect(hapticCalls.last.arguments, 'HapticFeedbackType.selectionClick');
        expect(find.text('Pollen'), findsOneWidget);
        await tester.tapAt(const Offset(8, 8));
        await tester.pumpAndSettle();
        expect(find.text('Pollen'), findsNothing);

        await tester.tap(
          find.byKey(const ValueKey('huddle-participant-avatar-self')),
        );
        await tester.pumpAndSettle();
        expect(find.text('You'), findsNothing);
        expect(
          find.byKey(
            const ValueKey('huddle-participant-spotlight-avatar-self'),
          ),
          findsNothing,
        );
        expect(hapticCalls, hasLength(2));
        final selfSemantics = tester.widget<Semantics>(
          find.byWidgetPredicate(
            (widget) => widget is Semantics && widget.properties.label == 'You',
          ),
        );
        expect(selfSemantics.properties.button, isFalse);
        expect(selfSemantics.properties.onTap, isNull);
        expect(selfSemantics.properties.hint, isNull);
        hapticCalls.clear();
        expect(find.text('Connected'), findsNothing);
        expect(find.text('Waiting for remote audio'), findsNothing);
        expect(find.text('Microphone muted'), findsNothing);

        expect(find.byKey(const ValueKey('huddle-more')), findsNothing);
        expect(find.byKey(const ValueKey('huddle-end')), findsNothing);
        expect(find.text('End for everyone'), findsNothing);

        transport.emitRemoteAudio();
        await tester.pump();
        expect(
          find.bySemanticsLabel(RegExp(r'Miles, speaking')),
          findsOneWidget,
        );
        final speakingHalo = tester.widget<Container>(
          find.byKey(const ValueKey('huddle-speaking-halo-desktop')),
        );
        final speakingHaloDecoration =
            speakingHalo.decoration! as BoxDecoration;
        expect(speakingHaloDecoration.border, isNull);
        expect(speakingHaloDecoration.color?.a, closeTo(0.07, 0.001));
        await tester.pump(const Duration(milliseconds: 70));
        final mediumScaleMidTransition = tester
            .widget<Transform>(
              find.byKey(const ValueKey('huddle-speaking-halo-scale-desktop')),
            )
            .transform
            .storage
            .first;
        expect(mediumScaleMidTransition, greaterThan(1));
        expect(mediumScaleMidTransition, lessThan(1.772));
        await tester.pump(const Duration(milliseconds: 70));
        final mediumSpeakingScale = tester
            .widget<Transform>(
              find.byKey(const ValueKey('huddle-speaking-halo-scale-desktop')),
            )
            .transform
            .storage
            .first;
        expect(mediumSpeakingScale, closeTo(1.772, 0.01));

        transport.emitRemoteAudio(levelDbov: -10, sequence: 2);
        await tester.pump(const Duration(milliseconds: 50));
        await tester.pump(const Duration(milliseconds: 70));
        final loudScaleMidTransition = tester
            .widget<Transform>(
              find.byKey(const ValueKey('huddle-speaking-halo-scale-desktop')),
            )
            .transform
            .storage
            .first;
        expect(loudScaleMidTransition, greaterThan(mediumSpeakingScale));
        expect(loudScaleMidTransition, lessThan(2.291));
        await tester.pump(const Duration(milliseconds: 70));
        final loudSpeakingScale = tester
            .widget<Transform>(
              find.byKey(const ValueKey('huddle-speaking-halo-scale-desktop')),
            )
            .transform
            .storage
            .first;
        expect(loudSpeakingScale, closeTo(2.291, 0.01));
        expect(loudSpeakingScale, greaterThan(mediumSpeakingScale));
        expect(loudSpeakingScale, lessThanOrEqualTo(2.55));

        expect(
          find.descendant(
            of: find.byKey(const ValueKey('huddle-speaker-toggle')),
            matching: find.byIcon(LucideIcons.volume2),
          ),
          findsOneWidget,
        );
        final speakerIcon = find.descendant(
          of: find.byKey(const ValueKey('huddle-speaker-toggle')),
          matching: find.byIcon(LucideIcons.volume2),
        );
        final leaveIcon = find.descendant(
          of: find.byKey(const ValueKey('huddle-leave')),
          matching: find.byIcon(LucideIcons.phoneOff),
        );
        expect(tester.widget<Icon>(speakerIcon).size, 28);
        expect(tester.widget<Icon>(leaveIcon).size, 28);
        final inactiveSpeakerButton = tester.widget<IconButton>(
          find.descendant(
            of: find.byKey(const ValueKey('huddle-speaker-toggle')),
            matching: find.byType(IconButton),
          ),
        );
        expect(
          tester
              .widget<Semantics>(
                find
                    .descendant(
                      of: find.byKey(const ValueKey('huddle-speaker-toggle')),
                      matching: find.byType(Semantics),
                    )
                    .first,
              )
              .properties
              .toggled,
          isFalse,
        );
        final inactiveSpeakerFill = inactiveSpeakerButton.style?.backgroundColor
            ?.resolve(const <WidgetState>{});
        await tester.tap(find.byKey(const ValueKey('huddle-speaker-toggle')));
        await tester.pump();
        expect(hapticCalls, hasLength(1));
        expect(hapticCalls.last.arguments, 'HapticFeedbackType.selectionClick');
        expect(
          find.descendant(
            of: find.byKey(const ValueKey('huddle-speaker-toggle')),
            matching: find.byIcon(LucideIcons.volume2),
          ),
          findsOneWidget,
        );
        final activeSpeakerButton = tester.widget<IconButton>(
          find.descendant(
            of: find.byKey(const ValueKey('huddle-speaker-toggle')),
            matching: find.byType(IconButton),
          ),
        );
        expect(
          activeSpeakerButton.style?.backgroundColor?.resolve(
            const <WidgetState>{},
          ),
          isNot(inactiveSpeakerFill),
        );
        expect(
          tester
              .widget<Semantics>(
                find
                    .descendant(
                      of: find.byKey(const ValueKey('huddle-speaker-toggle')),
                      matching: find.byType(Semantics),
                    )
                    .first,
              )
              .properties
              .toggled,
          isTrue,
        );

        expect(
          find.descendant(
            of: find.byKey(const ValueKey('huddle-mute-toggle')),
            matching: find.byIcon(LucideIcons.mic),
          ),
          findsOneWidget,
        );
        expect(
          tester
              .widget<Icon>(
                find.descendant(
                  of: find.byKey(const ValueKey('huddle-mute-toggle')),
                  matching: find.byIcon(LucideIcons.mic),
                ),
              )
              .size,
          28,
        );
        await tester.tap(find.byKey(const ValueKey('huddle-mute-toggle')));
        await tester.pump();
        expect(hapticCalls, hasLength(2));
        expect(hapticCalls.last.arguments, 'HapticFeedbackType.selectionClick');
        expect(
          find.descendant(
            of: find.byKey(const ValueKey('huddle-mute-toggle')),
            matching: find.byIcon(LucideIcons.micOff),
          ),
          findsOneWidget,
        );
        expect(
          tester
              .widget<Semantics>(
                find
                    .descendant(
                      of: find.byKey(const ValueKey('huddle-mute-toggle')),
                      matching: find.byType(Semantics),
                    )
                    .first,
              )
              .properties
              .toggled,
          isTrue,
        );

        final emojiIcon = find.descendant(
          of: find.byKey(const ValueKey('huddle-emoji-reactions')),
          matching: find.byIcon(LucideIcons.smilePlus),
        );
        expect(emojiIcon, findsOneWidget);
        expect(tester.widget<Icon>(emojiIcon).size, 28);
        expect(find.bySemanticsLabel('Emoji reactions'), findsOneWidget);
        final huddleContainer = ProviderScope.containerOf(
          tester.element(find.byType(MobileHuddleShell)),
        );
        final localBurstController = huddleContainer.read(
          emojiBurstControllerProvider,
        )..clear();
        final localAvatarCenter = tester.getCenter(
          find.byKey(const ValueKey('huddle-speaking-ring-self')),
        );
        await tester.tap(find.byKey(const ValueKey('huddle-emoji-reactions')));
        await tester.pump(const Duration(milliseconds: 500));
        expect(hapticCalls, hasLength(3));
        expect(hapticCalls.last.arguments, 'HapticFeedbackType.selectionClick');
        expect(find.byType(EmojiPickerSheet), findsOneWidget);
        tester
            .widget<EmojiPickerSheet>(find.byType(EmojiPickerSheet))
            .onSelect('🎉');
        await tester.pump();
        expect(localBurstController.debugLastBurstOrigin, localAvatarCenter);
        await tester.pump(const Duration(milliseconds: 500));

        await tester.tap(find.byKey(const ValueKey('huddle-minimize')));
        await tester.pumpAndSettle();
        expect(hapticCalls, hasLength(4));
        expect(hapticCalls.last.arguments, 'HapticFeedbackType.selectionClick');
        expect(find.widgetWithText(FilledButton, 'Open'), findsOneWidget);
        expect(
          tester
              .widget<AnimatedPositioned>(
                find.byKey(
                  const ValueKey('mobile-huddle-app-surface-position'),
                ),
              )
              .bottom,
          80,
        );
        final appSurface = tester.widget<AnimatedContainer>(
          find.byKey(const ValueKey('mobile-huddle-app-surface')),
        );
        final appSurfaceDecoration = appSurface.decoration! as BoxDecoration;
        final appSurfaceRadius =
            appSurfaceDecoration.borderRadius! as BorderRadius;
        expect(appSurfaceRadius.bottomLeft.x, 24);
        expect(appSurfaceRadius.bottomRight.x, 24);
        expect(
          tester.getSize(find.byKey(const ValueKey('huddle-drawer-expand'))),
          const Size.square(64),
        );
        expect(
          tester.getSize(
            find.byKey(const ValueKey('huddle-drawer-speaker-toggle')),
          ),
          const Size.square(64),
        );
        expect(
          tester.getSize(
            find.byKey(const ValueKey('huddle-drawer-mute-toggle')),
          ),
          const Size.square(64),
        );
        expect(
          tester.getSize(find.byKey(const ValueKey('huddle-drawer-leave'))),
          const Size.square(64),
        );
        expect(
          find.descendant(
            of: find.byKey(const ValueKey('huddle-drawer-expand')),
            matching: find.byIcon(LucideIcons.chevronUp),
          ),
          findsOneWidget,
        );
        expect(
          find.descendant(
            of: find.byKey(const ValueKey('huddle-drawer-mute-toggle')),
            matching: find.byIcon(LucideIcons.micOff),
          ),
          findsOneWidget,
        );
        expect(find.bySemanticsLabel('Unmute'), findsOneWidget);

        await tester.tap(
          find.byKey(const ValueKey('huddle-drawer-speaker-toggle')),
        );
        await tester.pump();
        expect(hapticCalls, hasLength(5));
        expect(hapticCalls.last.arguments, 'HapticFeedbackType.selectionClick');

        await tester.tap(
          find.byKey(const ValueKey('huddle-drawer-mute-toggle')),
        );
        await tester.pump();
        expect(hapticCalls, hasLength(6));
        expect(hapticCalls.last.arguments, 'HapticFeedbackType.selectionClick');
        expect(
          tester
              .widget<AnimatedPositioned>(
                find.byKey(
                  const ValueKey('mobile-huddle-app-surface-position'),
                ),
              )
              .bottom,
          80,
        );

        final drawerRect = tester.getRect(
          find.byKey(const ValueKey('mobile-huddle-drawer')),
        );
        final drawerOffset = tester.widget<Transform>(
          find.byKey(const ValueKey('huddle-drawer-control-offset')),
        );
        expect(drawerOffset.transform.storage[13], -8);
        final primaryControlsRect = tester.getRect(
          find.byKey(const ValueKey('huddle-drawer-primary-controls')),
        );
        final leaveRect = tester.getRect(
          find.byKey(const ValueKey('huddle-drawer-leave')),
        );
        expect(
          primaryControlsRect.left,
          closeTo(drawerRect.left + Grid.gutter, 0.01),
        );
        expect(leaveRect.right, closeTo(drawerRect.right - Grid.gutter, 0.01));
        final drawerControlCenter = drawerRect.center.dy;
        for (final key in const [
          ValueKey('huddle-drawer-expand'),
          ValueKey('huddle-drawer-speaker-toggle'),
          ValueKey('huddle-drawer-mute-toggle'),
          ValueKey('huddle-drawer-leave'),
        ]) {
          expect(
            tester.getCenter(find.byKey(key)).dy,
            closeTo(drawerControlCenter - 8, 0.01),
          );
        }
        await tester.tap(find.byKey(const ValueKey('huddle-drawer-expand')));
        await tester.pumpAndSettle();
        expect(hapticCalls, hasLength(7));
        expect(hapticCalls.last.arguments, 'HapticFeedbackType.selectionClick');
        expect(find.byKey(const ValueKey('huddle-minimize')), findsOneWidget);
        expect(
          tester
              .widget<AnimatedPositioned>(
                find.byKey(
                  const ValueKey('mobile-huddle-app-surface-position'),
                ),
              )
              .bottom,
          0,
        );

        await tester.tap(find.byKey(const ValueKey('huddle-minimize')));
        await tester.pumpAndSettle();
        expect(hapticCalls, hasLength(8));
        expect(hapticCalls.last.arguments, 'HapticFeedbackType.selectionClick');
        await tester.tap(find.byKey(const ValueKey('huddle-drawer-leave')));
        await tester.pump();
        expect(hapticCalls, hasLength(9));
        expect(hapticCalls.last.arguments, 'HapticFeedbackType.selectionClick');
        expect(
          tester
              .widget<AnimatedPositioned>(
                find.byKey(
                  const ValueKey('mobile-huddle-app-surface-position'),
                ),
              )
              .bottom,
          0,
        );
        for (
          var attempt = 0;
          attempt < 100 && leftChannelId == null;
          attempt++
        ) {
          await tester.pump();
        }
        expect(media.state.phase, HuddleMediaPhase.stopped);
        expect(leftChannelId, _huddleChannelId);
      },
    );

    testWidgets(
      'caps a dense Huddle roster at ten avatars with an overflow count',
      (tester) async {
        const membershipAgentPubkey = 'membership-agent';
        tester.view.physicalSize = const Size(390, 844);
        tester.view.devicePixelRatio = 1;
        addTearDown(tester.view.resetPhysicalSize);
        addTearDown(tester.view.resetDevicePixelRatio);

        final now = DateTime.now().millisecondsSinceEpoch ~/ 1000;
        // The relay caps a room at 25 peers, so 24 remotes plus the local
        // participant exercises the densest supported call.
        final remotePubkeys = List.generate(24, (index) => 'guest-$index');
        final transport = _HuddleTestTransport(
          peers: {
            0: const HuddlePeer(pubkey: 'self', peerIndex: 0, epoch: 0),
            for (var index = 0; index < remotePubkeys.length; index++)
              index + 1: HuddlePeer(
                pubkey: remotePubkeys[index],
                peerIndex: index + 1,
                epoch: 0,
              ),
          },
        );

        await tester.pumpWidget(
          _buildTestable(
            messages: [
              _huddleMsg(
                id: 'dense-call-layout',
                kind: EventKind.huddleStarted,
                pubkey: 'self',
                createdAt: now,
              ),
            ],
            users: {
              'self': const UserProfile(pubkey: 'self', displayName: 'Self'),
              membershipAgentPubkey: const UserProfile(
                pubkey: membershipAgentPubkey,
                displayName: 'Membership agent',
              ),
              for (final pubkey in remotePubkeys)
                pubkey: UserProfile(pubkey: pubkey, displayName: pubkey),
            },
            huddleMembers: [
              for (final pubkey in remotePubkeys)
                ChannelMember(
                  pubkey: pubkey,
                  role: 'member',
                  joinedAt: DateTime(2025),
                ),
              ChannelMember(
                pubkey: membershipAgentPubkey,
                role: 'bot',
                joinedAt: DateTime(2025),
              ),
            ],
            relayConfigNotifier: _HuddleRelayConfigNotifier(),
            huddleCurrentPubkey: 'self',
            huddleMediaFactory: _HuddleTestMedia.new,
            huddleTransportFactory: (_) => transport,
          ),
        );
        await tester.pumpAndSettle();

        await tester.tap(find.widgetWithText(FilledButton, 'Join'));
        await tester.pumpAndSettle();

        final remoteRegion = find.byKey(
          const ValueKey('huddle-remote-participant-region'),
        );
        expect(
          tester.widget<FractionallySizedBox>(remoteRegion).heightFactor,
          0.58,
        );
        expect(
          find.descendant(
            of: remoteRegion,
            matching: find.byType(SingleChildScrollView),
          ),
          findsNothing,
        );
        for (final pubkey in remotePubkeys.take(10)) {
          expect(
            find.byKey(ValueKey('huddle-participant-avatar-$pubkey')),
            findsOneWidget,
          );
        }
        for (final pubkey in remotePubkeys.skip(10)) {
          expect(
            find.byKey(ValueKey('huddle-participant-avatar-$pubkey')),
            findsNothing,
          );
        }
        expect(
          find.byKey(const ValueKey('huddle-participant-overflow')),
          findsOneWidget,
        );
        expect(find.text('+15'), findsOneWidget);

        await tester.tap(
          find.byKey(const ValueKey('huddle-participant-overflow')),
        );
        await tester.pumpAndSettle();

        expect(
          find.byKey(const ValueKey('huddle-participant-roster')),
          findsOneWidget,
        );
        expect(find.text('15 more people'), findsOneWidget);
        expect(
          find.byKey(const ValueKey('huddle-participant-roster-row-guest-10')),
          findsOneWidget,
        );
        expect(
          find.byKey(const ValueKey('huddle-participant-roster-row-guest-0')),
          findsNothing,
        );
        await tester.scrollUntilVisible(
          find.byKey(const ValueKey('huddle-participant-roster-row-guest-23')),
          200,
          scrollable: find.descendant(
            of: find.byKey(const ValueKey('huddle-participant-roster-list')),
            matching: find.byType(Scrollable),
          ),
        );
        expect(
          find.byKey(const ValueKey('huddle-participant-roster-row-guest-23')),
          findsOneWidget,
        );
        final membershipAgentRow = find.byKey(
          const ValueKey(
            'huddle-participant-roster-row-$membershipAgentPubkey',
          ),
        );
        await tester.scrollUntilVisible(
          membershipAgentRow,
          200,
          scrollable: find.descendant(
            of: find.byKey(const ValueKey('huddle-participant-roster-list')),
            matching: find.byType(Scrollable),
          ),
        );
        expect(membershipAgentRow, findsOneWidget);
        await tester.tapAt(const Offset(8, 8));
        await tester.pumpAndSettle();
        expect(
          find.byKey(const ValueKey('huddle-participant-roster')),
          findsNothing,
        );

        final firstRemoteRing = find.byKey(
          const ValueKey('huddle-speaking-ring-guest-0'),
        );
        final firstRemoteAvatar = find
            .descendant(
              of: firstRemoteRing,
              matching: find.byType(CircleAvatar),
            )
            .first;
        expect(tester.getSize(firstRemoteAvatar).width, lessThan(104));
        expect(tester.takeException(), isNull);
      },
    );

    testWidgets(
      'bursts remote Huddle reactions and ignores the local relay echo',
      (tester) async {
        final now = DateTime.now().millisecondsSinceEpoch ~/ 1000;
        final relaySession = _HuddleReactionRelaySession();

        await tester.pumpWidget(
          _buildTestable(
            messages: [
              _huddleMsg(
                id: 'reaction-call',
                kind: EventKind.huddleStarted,
                pubkey: 'desktop',
                createdAt: now,
              ),
            ],
            users: const {
              'desktop': UserProfile(pubkey: 'desktop', displayName: 'Miles'),
              'self': UserProfile(pubkey: 'self', displayName: 'Self'),
            },
            relayConfigNotifier: _HuddleRelayConfigNotifier(),
            relaySessionNotifier: relaySession,
            huddleCurrentPubkey: 'self',
            huddleMediaFactory: _HuddleTestMedia.new,
            huddleTransportFactory: (_) => _HuddleTestTransport(),
          ),
        );
        await tester.pumpAndSettle();

        await tester.tap(find.widgetWithText(FilledButton, 'Join'));
        await tester.pumpAndSettle();
        for (
          var attempt = 0;
          attempt < 20 && relaySession.reactionFilter == null;
          attempt++
        ) {
          await tester.pump();
        }

        expect(
          relaySession.reactionFilter?.kinds,
          contains(EventKind.huddleReaction),
        );
        expect(relaySession.reactionFilter?.tags['#h'], [_huddleChannelId]);
        expect(relaySession.reactionFilter?.since, isNotNull);

        final container = ProviderScope.containerOf(
          tester.element(find.byType(MobileHuddleShell)),
        );
        final burstController = container.read(emojiBurstControllerProvider);
        expect(burstController.hasParticles, isFalse);
        final remoteAvatarCenter = tester.getCenter(
          find.byKey(const ValueKey('huddle-speaking-ring-desktop')),
        );

        relaySession.emitReaction(pubkey: 'self', emoji: '🎉');
        await tester.pump();
        expect(burstController.hasParticles, isFalse);

        relaySession.emitReaction(pubkey: 'desktop', emoji: '🎉');
        await tester.pump();
        expect(burstController.hasParticles, isTrue);
        expect(burstController.debugLastBurstOrigin, remoteAvatarCenter);
      },
    );

    testWidgets(
      'centers a solo participant and moves them when another person joins',
      (tester) async {
        const guestPubkey = 'guest';
        final now = DateTime.now().millisecondsSinceEpoch ~/ 1000;
        final transport = _HuddleTestTransport(
          peers: const {2: HuddlePeer(pubkey: 'self', peerIndex: 2, epoch: 0)},
        );

        await tester.pumpWidget(
          _buildTestable(
            messages: [
              _huddleMsg(
                id: 'solo-call-motion',
                kind: EventKind.huddleStarted,
                pubkey: 'self',
                createdAt: now,
              ),
            ],
            users: const {
              'self': UserProfile(pubkey: 'self', displayName: 'Self'),
              guestPubkey: UserProfile(
                pubkey: guestPubkey,
                displayName: 'Guest',
              ),
            },
            relayConfigNotifier: _HuddleRelayConfigNotifier(),
            relaySessionNotifier: _ReconnectingRelaySession(),
            huddleCurrentPubkey: 'self',
            huddleMediaFactory: _HuddleTestMedia.new,
            huddleTransportFactory: (_) => transport,
          ),
        );
        await tester.pumpAndSettle();
        await tester.tap(find.widgetWithText(FilledButton, 'Join'));
        await tester.pumpAndSettle();

        final stage = find.byKey(const ValueKey('huddle-participant-stage'));
        final localAvatar = find.byKey(
          const ValueKey('huddle-speaking-ring-self'),
        );
        final soloCenter = tester.getCenter(localAvatar).dy;
        expect(soloCenter, closeTo(tester.getCenter(stage).dy, 1));

        transport.emitPeerJoin(
          const HuddlePeer(pubkey: guestPubkey, peerIndex: 1, epoch: 0),
        );
        await tester.pump();
        await tester.pump(const Duration(milliseconds: 130));
        final movingCenter = tester.getCenter(localAvatar).dy;
        expect(movingCenter, greaterThan(soloCenter));

        await tester.pumpAndSettle();
        final occupiedCenter = tester.getCenter(localAvatar).dy;
        expect(occupiedCenter, greaterThan(movingCenter));
        expect(
          tester
              .getCenter(
                find.byKey(const ValueKey('huddle-speaking-ring-$guestPubkey')),
              )
              .dy,
          lessThan(occupiedCenter),
        );
      },
    );

    testWidgets(
      'shows admitted agents from membership before their audio peer joins',
      (tester) async {
        const addedMemberPubkey =
            'aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa';
        final now = DateTime.now().millisecondsSinceEpoch ~/ 1000;
        final membersNotifier = _MutableHuddleMembersNotifier(const []);
        final transport = _HuddleTestTransport();

        await tester.pumpWidget(
          _buildTestable(
            messages: [
              _huddleMsg(
                id: 'authoritative-live-roster',
                kind: EventKind.huddleStarted,
                pubkey: 'self',
                createdAt: now,
              ),
            ],
            users: const {
              'desktop': UserProfile(pubkey: 'desktop', displayName: 'Miles'),
              'self': UserProfile(pubkey: 'self', displayName: 'Self'),
              addedMemberPubkey: UserProfile(
                pubkey: addedMemberPubkey,
                displayName: 'Added member',
              ),
            },
            huddleMembersNotifier: membersNotifier,
            relayConfigNotifier: _HuddleRelayConfigNotifier(),
            relaySessionNotifier: _ReconnectingRelaySession(),
            huddleCurrentPubkey: 'self',
            huddleMediaFactory: _HuddleTestMedia.new,
            huddleTransportFactory: (_) => transport,
          ),
        );
        await tester.pumpAndSettle();
        await tester.tap(find.widgetWithText(FilledButton, 'Join'));
        await tester.pumpAndSettle();

        final desktopAvatar = find.byKey(
          const ValueKey('huddle-speaking-ring-desktop'),
        );
        final initialDesktopCenter = tester.getCenter(desktopAvatar);

        membersNotifier.replace([
          ChannelMember(
            pubkey: addedMemberPubkey,
            role: 'bot',
            joinedAt: DateTime(2025),
          ),
        ]);
        await tester.pump();
        await tester.pump(const Duration(milliseconds: 100));
        expect(
          find.byKey(
            const ValueKey('huddle-participant-avatar-$addedMemberPubkey'),
          ),
          findsOneWidget,
        );

        final addedMemberScale = find.byKey(
          const ValueKey('huddle-participant-entry-scale-$addedMemberPubkey'),
        );
        expect(
          tester.widget<Transform>(addedMemberScale).transform.storage[0],
          closeTo(0.72, 0.01),
        );
        await tester.pump(const Duration(milliseconds: 120));
        final movingDesktopCenter = tester.getCenter(desktopAvatar);
        expect(
          tester.widget<Transform>(addedMemberScale).transform.storage[0],
          greaterThan(0.72),
        );
        expect(
          (movingDesktopCenter - initialDesktopCenter).distance,
          greaterThan(1),
        );
        expect(
          find.byKey(
            const ValueKey('huddle-participant-avatar-$addedMemberPubkey'),
          ),
          findsOneWidget,
        );
        expect(
          find.byKey(const ValueKey('huddle-participant-avatar-desktop')),
          findsOneWidget,
        );

        await tester.tap(
          find.byKey(
            const ValueKey('huddle-participant-avatar-$addedMemberPubkey'),
          ),
        );
        await tester.pumpAndSettle();
        expect(
          find.byKey(const ValueKey('huddle-participant-spotlight')),
          findsOneWidget,
        );
        expect(find.text('Added member'), findsOneWidget);
        await tester.tapAt(const Offset(8, 8));
        await tester.pumpAndSettle();

        await tester.pumpAndSettle();
        final settledDesktopCenter = tester.getCenter(desktopAvatar);
        expect(
          (settledDesktopCenter - movingDesktopCenter).distance,
          greaterThan(1),
        );
        expect(
          (settledDesktopCenter - initialDesktopCenter).distance,
          greaterThan(1),
        );
        expect(
          tester.widget<Transform>(addedMemberScale).transform.storage[0],
          closeTo(1, 0.01),
        );

        // The avatar reflects logical membership, so an audio connection ending
        // does not remove the agent. Removing its membership does.
        transport.emitPeerJoin(
          const HuddlePeer(pubkey: addedMemberPubkey, peerIndex: 3, epoch: 0),
        );
        await tester.pump();
        transport.emitPeerLeave(3);
        await tester.pump();
        expect(
          find.byKey(
            const ValueKey('huddle-participant-avatar-$addedMemberPubkey'),
          ),
          findsOneWidget,
        );
        await tester.pumpAndSettle();
        expect(
          find.byKey(
            const ValueKey('huddle-participant-avatar-$addedMemberPubkey'),
          ),
          findsOneWidget,
        );

        membersNotifier.replace(const []);
        await tester.pumpAndSettle();
        expect(
          find.byKey(
            const ValueKey('huddle-participant-avatar-$addedMemberPubkey'),
          ),
          findsNothing,
        );
      },
    );

    testWidgets(
      'does not show admitted human membership without an audio peer',
      (tester) async {
        const invitedHumanPubkey =
            'bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb';
        final now = DateTime.now().millisecondsSinceEpoch ~/ 1000;
        final membersNotifier = _MutableHuddleMembersNotifier(const []);

        await tester.pumpWidget(
          _buildTestable(
            messages: [
              _huddleMsg(
                id: 'human-membership-is-not-audio-presence',
                kind: EventKind.huddleStarted,
                pubkey: 'self',
                createdAt: now,
              ),
            ],
            users: const {
              'desktop': UserProfile(pubkey: 'desktop', displayName: 'Desktop'),
              'self': UserProfile(pubkey: 'self', displayName: 'Self'),
              invitedHumanPubkey: UserProfile(
                pubkey: invitedHumanPubkey,
                displayName: 'Invited human',
              ),
            },
            huddleMembersNotifier: membersNotifier,
            relayConfigNotifier: _HuddleRelayConfigNotifier(),
            huddleCurrentPubkey: 'self',
            huddleMediaFactory: _HuddleTestMedia.new,
            huddleTransportFactory: (_) => _HuddleTestTransport(),
          ),
        );
        await tester.pumpAndSettle();
        await tester.tap(find.widgetWithText(FilledButton, 'Join'));
        await tester.pumpAndSettle();

        membersNotifier.replace([
          ChannelMember(
            pubkey: invitedHumanPubkey,
            role: 'member',
            joinedAt: DateTime(2025),
          ),
        ]);
        await tester.pump();
        await tester.pump(const Duration(milliseconds: 100));

        expect(
          find.byKey(
            const ValueKey('huddle-participant-avatar-$invitedHumanPubkey'),
          ),
          findsNothing,
        );
      },
    );

    testWidgets('top-right call end leaves audio and the backing channel', (
      tester,
    ) async {
      final now = DateTime.now().millisecondsSinceEpoch ~/ 1000;
      final media = _HuddleTestMedia();
      final transport = _HuddleTestTransport();
      String? leftChannelId;
      final hapticCalls = <MethodCall>[];
      tester.binding.defaultBinaryMessenger.setMockMethodCallHandler(
        SystemChannels.platform,
        (call) async {
          if (call.method == 'HapticFeedback.vibrate') {
            hapticCalls.add(call);
          }
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
            _huddleMsg(
              id: 'active-call-leave',
              kind: EventKind.huddleStarted,
              pubkey: 'desktop',
              createdAt: now,
            ),
          ],
          users: const {
            'desktop': UserProfile(pubkey: 'desktop'),
            'self': UserProfile(pubkey: 'self'),
          },
          relayConfigNotifier: _HuddleRelayConfigNotifier(),
          relaySessionNotifier: _ReconnectingRelaySession(),
          huddleCurrentPubkey: 'self',
          huddleHumanCountLoader: (_) async => 2,
          huddleMediaFactory: () => media,
          huddleTransportFactory: (_) => transport,
          createChannelActions: (ref) => _FakeChannelActions(
            ref,
            onLeaveChannel: (channelId) async => leftChannelId = channelId,
          ),
        ),
      );
      await tester.pumpAndSettle();

      await tester.tap(find.widgetWithText(FilledButton, 'Join'));
      await tester.pumpAndSettle();
      final hangup = find.byKey(const ValueKey('huddle-leave'));
      expect(
        tester.getCenter(hangup).dy,
        lessThan(
          tester.getCenter(find.byKey(const ValueKey('huddle-mute-toggle'))).dy,
        ),
      );

      await tester.tap(hangup);
      await tester.pump();
      expect(hapticCalls, hasLength(1));
      expect(hapticCalls.single.arguments, 'HapticFeedbackType.selectionClick');
      for (var attempt = 0; attempt < 100 && leftChannelId == null; attempt++) {
        await tester.pump();
      }
      for (var attempt = 0; attempt < 20; attempt++) {
        await tester.pump();
      }
      await tester.pump(const Duration(milliseconds: 300));
      await tester.pump(const Duration(milliseconds: 300));
      await tester.pump();

      expect(leftChannelId, '8d764100-fd8f-44cf-9c98-6d8fbd739b8c');
    });

    testWidgets('top-right call end auto-ends for the last human', (
      tester,
    ) async {
      final now = DateTime.now().millisecondsSinceEpoch ~/ 1000;
      final media = _HuddleTestMedia();
      final transport = _HuddleTestTransport();
      final relaySession = _ReconnectingRelaySession();
      String? leftChannelId;
      String? archivedChannelId;

      await tester.pumpWidget(
        _buildTestable(
          messages: [
            _huddleMsg(
              id: 'last-human-call-leave',
              kind: EventKind.huddleStarted,
              pubkey: 'desktop',
              createdAt: now,
            ),
          ],
          users: const {
            'desktop': UserProfile(pubkey: 'desktop'),
            'agent': UserProfile(pubkey: 'agent', displayName: 'Pollen'),
            'self': UserProfile(pubkey: 'self'),
          },
          relayConfigNotifier: _HuddleRelayConfigNotifier(),
          relaySessionNotifier: relaySession,
          huddleCurrentPubkey: 'self',
          huddleMembers: [
            ChannelMember(
              pubkey: 'self',
              role: 'member',
              joinedAt: DateTime(2025),
            ),
            ChannelMember(
              pubkey: 'agent',
              role: 'bot',
              joinedAt: DateTime(2025),
            ),
          ],
          huddleMediaFactory: () => media,
          huddleTransportFactory: (_) => transport,
          createChannelActions: (ref) => _FakeChannelActions(
            ref,
            onLeaveChannel: (channelId) async => leftChannelId = channelId,
            onArchiveChannel: (channelId) async =>
                archivedChannelId = channelId,
          ),
        ),
      );
      await tester.pumpAndSettle();

      await tester.tap(find.widgetWithText(FilledButton, 'Join'));
      await tester.pumpAndSettle();
      relaySession.connect();
      await tester.pump();
      await tester.tap(find.byKey(const ValueKey('huddle-leave')));
      await tester.pump();
      for (
        var attempt = 0;
        attempt < 100 && archivedChannelId == null;
        attempt++
      ) {
        await tester.pump();
      }
      for (var attempt = 0; attempt < 20; attempt++) {
        await tester.pump();
      }
      await tester.pump(const Duration(milliseconds: 300));
      await tester.pump(const Duration(milliseconds: 300));
      await tester.pump();

      expect(leftChannelId, isNull);
      expect(archivedChannelId, '8d764100-fd8f-44cf-9c98-6d8fbd739b8c');
      expect(relaySession.publishedKinds, contains(EventKind.huddleEnded));
    });

    testWidgets(
      'duplicate hangup still completes the admitted lifecycle after local teardown',
      (tester) async {
        final now = DateTime.now().millisecondsSinceEpoch ~/ 1000;
        final media = _HuddleTestMedia();
        final transport = _HuddleTestTransport();
        final humanCount = Completer<int>();
        final relaySession = _ReconnectingRelaySession();
        String? archivedChannelId;

        await tester.pumpWidget(
          _buildTestable(
            messages: [
              _huddleMsg(
                id: 'pending-member-lookup-leave',
                kind: EventKind.huddleStarted,
                pubkey: 'self',
                createdAt: now,
              ),
            ],
            users: const {'self': UserProfile(pubkey: 'self')},
            relayConfigNotifier: _HuddleRelayConfigNotifier(),
            relaySessionNotifier: relaySession,
            huddleCurrentPubkey: 'self',
            huddleHumanCountLoader: (_) => humanCount.future,
            huddleMediaFactory: () => media,
            huddleTransportFactory: (_) => transport,
            createChannelActions: (ref) => _FakeChannelActions(
              ref,
              onArchiveChannel: (channelId) async =>
                  archivedChannelId = channelId,
            ),
          ),
        );
        await tester.pumpAndSettle();

        await tester.tap(find.widgetWithText(FilledButton, 'Join'));
        await tester.pumpAndSettle();
        await tester.tap(find.byKey(const ValueKey('huddle-leave')));
        final huddleContainer = ProviderScope.containerOf(
          tester.element(find.byType(MobileHuddleShell)),
        );
        unawaited(
          huddleContainer.read(mobileHuddleControllerProvider.notifier).leave(),
        );
        for (var attempt = 0; attempt < 20; attempt++) {
          await tester.pump();
        }
        await tester.pump(const Duration(milliseconds: 300));

        expect(find.byKey(const ValueKey('huddle-leave')), findsNothing);
        expect(media.state.phase, HuddleMediaPhase.stopped);
        expect(archivedChannelId, isNull);

        humanCount.complete(1);
        for (
          var attempt = 0;
          attempt < 100 && archivedChannelId == null;
          attempt++
        ) {
          await tester.pump();
        }

        expect(archivedChannelId, _huddleChannelId);
        expect(relaySession.publishedKinds, contains(EventKind.huddleEnded));
      },
    );

    testWidgets(
      'rejected non-creator end preserves the later leave lifecycle',
      (tester) async {
        final now = DateTime.now().millisecondsSinceEpoch ~/ 1000;
        String? leftChannelId;

        await tester.pumpWidget(
          _buildTestable(
            messages: [
              _huddleMsg(
                id: 'non-creator-end',
                kind: EventKind.huddleStarted,
                pubkey: 'desktop',
                createdAt: now,
              ),
            ],
            users: const {
              'desktop': UserProfile(pubkey: 'desktop'),
              'self': UserProfile(pubkey: 'self'),
            },
            relayConfigNotifier: _HuddleRelayConfigNotifier(),
            relaySessionNotifier: _ReconnectingRelaySession(),
            huddleCurrentPubkey: 'self',
            huddleHumanCountLoader: (_) async => 2,
            huddleMediaFactory: _HuddleTestMedia.new,
            huddleTransportFactory: (_) => _HuddleTestTransport(),
            createChannelActions: (ref) => _FakeChannelActions(
              ref,
              onLeaveChannel: (channelId) async => leftChannelId = channelId,
            ),
          ),
        );
        await tester.pumpAndSettle();
        await tester.tap(find.widgetWithText(FilledButton, 'Join'));
        await tester.pumpAndSettle();

        final controller = ProviderScope.containerOf(
          tester.element(find.byType(MobileHuddleShell)),
        ).read(mobileHuddleControllerProvider.notifier);
        await expectLater(controller.end(), throwsStateError);
        await controller.leave();

        expect(leftChannelId, _huddleChannelId);
      },
    );

    testWidgets(
      'community transition cancels and awaits an in-flight Huddle start',
      (tester) async {
        final createGate = Completer<void>();
        final relaySession = _ReconnectingRelaySession(
          huddleCreatePublishGate: createGate.future,
        );
        String? archivedChannelId;

        await tester.pumpWidget(
          _buildTestable(
            messages: const [],
            users: const {'self': UserProfile(pubkey: 'self')},
            relayConfigNotifier: _HuddleRelayConfigNotifier(),
            relaySessionNotifier: relaySession,
            huddleCurrentPubkey: 'self',
            huddleMediaFactory: _HuddleTestMedia.new,
            huddleTransportFactory: (_) => _HuddleTestTransport(),
            createChannelActions: (ref) => _FakeChannelActions(
              ref,
              onArchiveChannel: (channelId) async =>
                  archivedChannelId = channelId,
            ),
          ),
        );
        await tester.pumpAndSettle();

        await tester.tap(find.byKey(const ValueKey('channel-huddle-button')));
        await relaySession.huddleCreatePublishStarted.future;
        var transitionCompleted = false;
        final transition =
            ProviderScope.containerOf(
              tester.element(find.byType(MobileHuddleShell)),
            ).read(communityTransitionProvider).run().then((_) {
              transitionCompleted = true;
            });
        await tester.pump();

        expect(transitionCompleted, isFalse);
        createGate.complete();
        await transition;

        expect(archivedChannelId, isNotNull);
        expect(
          relaySession.publishedKinds,
          isNot(contains(EventKind.huddleStarted)),
        );
        await tester.pump(const Duration(seconds: 1));
      },
    );

    testWidgets(
      'community transition leaves a newer call after background cleanup',
      (tester) async {
        final now = DateTime.now().millisecondsSinceEpoch ~/ 1000;
        final oldMedia = _HuddleTestMedia();
        final currentMedia = _HuddleTestMedia();
        final media = Queue<_HuddleTestMedia>.of([oldMedia, currentMedia]);
        final transports = Queue<_HuddleTestTransport>.of([
          _HuddleTestTransport(),
          _HuddleTestTransport(),
        ]);
        final oldHumanCount = Completer<int>();
        var humanCountCalls = 0;

        await tester.pumpWidget(
          _buildTestable(
            messages: [
              _huddleMsg(
                id: 'background-transition-call',
                kind: EventKind.huddleStarted,
                pubkey: 'desktop',
                createdAt: now,
              ),
            ],
            users: const {
              'desktop': UserProfile(pubkey: 'desktop'),
              'self': UserProfile(pubkey: 'self'),
            },
            relayConfigNotifier: _HuddleRelayConfigNotifier(),
            relaySessionNotifier: _ReconnectingRelaySession(),
            huddleCurrentPubkey: 'self',
            huddleHumanCountLoader: (_) {
              humanCountCalls++;
              return humanCountCalls == 1
                  ? oldHumanCount.future
                  : Future.value(2);
            },
            huddleMediaFactory: media.removeFirst,
            huddleTransportFactory: (_) => transports.removeFirst(),
          ),
        );
        await tester.pumpAndSettle();
        await tester.tap(find.widgetWithText(FilledButton, 'Join'));
        await tester.pumpAndSettle();

        final container = ProviderScope.containerOf(
          tester.element(find.byType(MobileHuddleShell)),
        );
        final lifecycle = container.read(appLifecycleProvider.notifier);
        expect(lifecycle, isA<_TestAppLifecycleNotifier>());
        (lifecycle as _TestAppLifecycleNotifier).setLifecycle(
          AppLifecycleState.paused,
        );
        await oldMedia.stopStarted.future;
        await tester.pump();
        await container
            .read(mobileHuddleControllerProvider.notifier)
            .join(
              parentChannelId: _channelId,
              ephemeralChannelId: _huddleChannelId,
              startedBy: 'desktop',
              startedEventId: 'background-transition-call',
            );

        var transitionCompleted = false;
        final transition = container
            .read(communityTransitionProvider)
            .run()
            .then((_) => transitionCompleted = true);
        await tester.pump();
        expect(transitionCompleted, isFalse);

        oldHumanCount.complete(2);
        await transition;

        expect(currentMedia.state.phase, HuddleMediaPhase.stopped);
        expect(container.read(huddleSessionProvider).isInSession, isFalse);
      },
    );

    testWidgets(
      'community transition awaits failed-session lifecycle cleanup',
      (tester) async {
        final now = DateTime.now().millisecondsSinceEpoch ~/ 1000;
        final media = _HuddleTestMedia();
        final humanCount = Completer<int>();
        String? leftChannelId;

        await tester.pumpWidget(
          _buildTestable(
            messages: [
              _huddleMsg(
                id: 'failed-call-transition',
                kind: EventKind.huddleStarted,
                pubkey: 'desktop',
                createdAt: now,
              ),
            ],
            users: const {
              'desktop': UserProfile(pubkey: 'desktop'),
              'self': UserProfile(pubkey: 'self'),
            },
            relayConfigNotifier: _HuddleRelayConfigNotifier(),
            relaySessionNotifier: _ReconnectingRelaySession(),
            huddleCurrentPubkey: 'self',
            huddleHumanCountLoader: (_) => humanCount.future,
            huddleMediaFactory: () => media,
            huddleTransportFactory: (_) => _HuddleTestTransport(),
            createChannelActions: (ref) => _FakeChannelActions(
              ref,
              onLeaveChannel: (channelId) async => leftChannelId = channelId,
            ),
          ),
        );
        await tester.pumpAndSettle();
        await tester.tap(find.widgetWithText(FilledButton, 'Join'));
        await tester.pumpAndSettle();

        media.emitFailure();
        await tester.pump();
        for (var attempt = 0; attempt < 20; attempt++) {
          await tester.pump();
        }
        final container = ProviderScope.containerOf(
          tester.element(find.byType(MobileHuddleShell)),
        );
        var transitionCompleted = false;
        final transition = container
            .read(communityTransitionProvider)
            .run()
            .then((_) => transitionCompleted = true);
        await tester.pump();

        expect(transitionCompleted, isFalse);
        expect(leftChannelId, isNull);
        humanCount.complete(2);
        await transition;

        expect(leftChannelId, _huddleChannelId);
      },
    );

    testWidgets(
      'background relay pause awaits failed-session lifecycle cleanup',
      (tester) async {
        final now = DateTime.now().millisecondsSinceEpoch ~/ 1000;
        final media = _HuddleTestMedia();
        final humanCount = Completer<int>();
        final relaySession = _ReconnectingRelaySession();
        String? leftChannelId;

        await tester.pumpWidget(
          _buildTestable(
            messages: [
              _huddleMsg(
                id: 'failed-call-background',
                kind: EventKind.huddleStarted,
                pubkey: 'desktop',
                createdAt: now,
              ),
            ],
            users: const {
              'desktop': UserProfile(pubkey: 'desktop'),
              'self': UserProfile(pubkey: 'self'),
            },
            relayConfigNotifier: _HuddleRelayConfigNotifier(),
            relaySessionNotifier: relaySession,
            huddleCurrentPubkey: 'self',
            huddleHumanCountLoader: (_) => humanCount.future,
            huddleMediaFactory: () => media,
            huddleTransportFactory: (_) => _HuddleTestTransport(),
            createChannelActions: (ref) => _FakeChannelActions(
              ref,
              onLeaveChannel: (channelId) async => leftChannelId = channelId,
            ),
          ),
        );
        await tester.pumpAndSettle();
        await tester.tap(find.widgetWithText(FilledButton, 'Join'));
        await tester.pumpAndSettle();

        media.emitFailure();
        await tester.pump();
        relaySession.onAppPaused();
        await tester.pump(const Duration(seconds: 5));
        await tester.pump();

        expect(relaySession.state.status, SessionStatus.reconnecting);
        expect(leftChannelId, isNull);

        humanCount.complete(2);
        for (
          var attempt = 0;
          attempt < 20 &&
              relaySession.state.status != SessionStatus.disconnected;
          attempt++
        ) {
          await tester.pump();
        }

        expect(leftChannelId, _huddleChannelId);
        expect(relaySession.state.status, SessionStatus.disconnected);
      },
    );

    testWidgets(
      'last-human leave superseded during count by another Huddle archives the old room',
      (tester) async {
        final now = DateTime.now().millisecondsSinceEpoch ~/ 1000;
        final humanCount = Completer<int>();
        final media = Queue<_HuddleTestMedia>.of([
          _HuddleTestMedia(),
          _HuddleTestMedia(),
        ]);
        final transports = Queue<_HuddleTestTransport>.of([
          _HuddleTestTransport(),
          _HuddleTestTransport(),
        ]);
        final relaySession = _ReconnectingRelaySession();
        String? archivedChannelId;

        await tester.pumpWidget(
          _buildTestable(
            messages: [
              _huddleMsg(
                id: 'different-huddle-during-count',
                kind: EventKind.huddleStarted,
                pubkey: 'self',
                createdAt: now,
              ),
            ],
            users: const {'self': UserProfile(pubkey: 'self')},
            relayConfigNotifier: _HuddleRelayConfigNotifier(),
            relaySessionNotifier: relaySession,
            huddleCurrentPubkey: 'self',
            huddleHumanCountLoader: (_) => humanCount.future,
            huddleMediaFactory: media.removeFirst,
            huddleTransportFactory: (_) => transports.removeFirst(),
            createChannelActions: (ref) => _FakeChannelActions(
              ref,
              onArchiveChannel: (channelId) async =>
                  archivedChannelId = channelId,
            ),
          ),
        );
        await tester.pumpAndSettle();
        await tester.tap(find.widgetWithText(FilledButton, 'Join'));
        await tester.pumpAndSettle();

        final container = ProviderScope.containerOf(
          tester.element(find.byType(MobileHuddleShell)),
        );
        final controller = container.read(
          mobileHuddleControllerProvider.notifier,
        );
        final staleLeave = controller.leave();
        await tester.pump();
        await controller.join(
          parentChannelId: _otherChannelId,
          ephemeralChannelId: _otherHuddleChannelId,
          startedBy: 'self',
          startedEventId: 'different-huddle-start',
        );
        humanCount.complete(1);
        await staleLeave;
        await tester.pump();

        expect(relaySession.publishedKinds, contains(EventKind.huddleEnded));
        expect(archivedChannelId, _huddleChannelId);
        expect(
          container.read(huddleSessionProvider).ephemeralChannelId,
          _otherHuddleChannelId,
        );
        await tester.pumpWidget(const SizedBox.shrink());
        await tester.pump(const Duration(milliseconds: 100));
      },
    );

    testWidgets(
      'last-human leave superseded during end publish by another Huddle archives the old room',
      (tester) async {
        final now = DateTime.now().millisecondsSinceEpoch ~/ 1000;
        final endPublishGate = Completer<void>();
        final media = Queue<_HuddleTestMedia>.of([
          _HuddleTestMedia(),
          _HuddleTestMedia(),
        ]);
        final transports = Queue<_HuddleTestTransport>.of([
          _HuddleTestTransport(),
          _HuddleTestTransport(),
        ]);
        final relaySession = _ReconnectingRelaySession(
          huddleEndPublishGate: endPublishGate.future,
        );
        String? archivedChannelId;

        await tester.pumpWidget(
          _buildTestable(
            messages: [
              _huddleMsg(
                id: 'different-huddle-during-end',
                kind: EventKind.huddleStarted,
                pubkey: 'self',
                createdAt: now,
              ),
            ],
            users: const {'self': UserProfile(pubkey: 'self')},
            relayConfigNotifier: _HuddleRelayConfigNotifier(),
            relaySessionNotifier: relaySession,
            huddleCurrentPubkey: 'self',
            huddleHumanCountLoader: (_) async => 1,
            huddleMediaFactory: media.removeFirst,
            huddleTransportFactory: (_) => transports.removeFirst(),
            createChannelActions: (ref) => _FakeChannelActions(
              ref,
              onArchiveChannel: (channelId) async =>
                  archivedChannelId = channelId,
            ),
          ),
        );
        await tester.pumpAndSettle();
        await tester.tap(find.widgetWithText(FilledButton, 'Join'));
        await tester.pumpAndSettle();

        final container = ProviderScope.containerOf(
          tester.element(find.byType(MobileHuddleShell)),
        );
        final controller = container.read(
          mobileHuddleControllerProvider.notifier,
        );
        final staleLeave = controller.leave();
        await relaySession.huddleEndPublishStarted.future;
        await controller.join(
          parentChannelId: _otherChannelId,
          ephemeralChannelId: _otherHuddleChannelId,
          startedBy: 'self',
          startedEventId: 'different-huddle-start',
        );
        endPublishGate.complete();
        await staleLeave;
        await tester.pump();

        expect(relaySession.publishedKinds, contains(EventKind.huddleEnded));
        expect(archivedChannelId, _huddleChannelId);
        expect(
          container.read(huddleSessionProvider).ephemeralChannelId,
          _otherHuddleChannelId,
        );
        await tester.pumpWidget(const SizedBox.shrink());
        await tester.pump(const Duration(milliseconds: 100));
      },
    );

    testWidgets(
      'last-human leave superseded by same-Huddle rejoin cannot archive it',
      (tester) async {
        final now = DateTime.now().millisecondsSinceEpoch ~/ 1000;
        final endPublishGate = Completer<void>();
        final media = Queue<_HuddleTestMedia>.of([
          _HuddleTestMedia(),
          _HuddleTestMedia(),
        ]);
        final transports = Queue<_HuddleTestTransport>.of([
          _HuddleTestTransport(),
          _HuddleTestTransport(),
        ]);
        final relaySession = _ReconnectingRelaySession(
          huddleEndPublishGate: endPublishGate.future,
        );
        String? archivedChannelId;

        await tester.pumpWidget(
          _buildTestable(
            messages: [
              _huddleMsg(
                id: 'last-human-rejoin',
                kind: EventKind.huddleStarted,
                pubkey: 'self',
                createdAt: now,
              ),
            ],
            users: const {'self': UserProfile(pubkey: 'self')},
            relayConfigNotifier: _HuddleRelayConfigNotifier(),
            relaySessionNotifier: relaySession,
            huddleCurrentPubkey: 'self',
            huddleHumanCountLoader: (_) async => 1,
            huddleMediaFactory: media.removeFirst,
            huddleTransportFactory: (_) => transports.removeFirst(),
            createChannelActions: (ref) => _FakeChannelActions(
              ref,
              onArchiveChannel: (channelId) async =>
                  archivedChannelId = channelId,
            ),
          ),
        );
        await tester.pumpAndSettle();
        await tester.tap(find.widgetWithText(FilledButton, 'Join'));
        await tester.pumpAndSettle();

        final container = ProviderScope.containerOf(
          tester.element(find.byType(MobileHuddleShell)),
        );
        final controller = container.read(
          mobileHuddleControllerProvider.notifier,
        );
        final staleLeave = controller.leave();
        await relaySession.huddleEndPublishStarted.future;
        final rejoin = controller.join(
          parentChannelId: _channelId,
          ephemeralChannelId: _huddleChannelId,
          startedBy: 'self',
          startedEventId: 'last-human-rejoin',
        );
        endPublishGate.complete();
        await staleLeave;
        await rejoin;
        await tester.pump();

        expect(relaySession.publishedKinds, contains(EventKind.huddleEnded));
        expect(archivedChannelId, isNull);
        expect(container.read(huddleSessionProvider).isConnected, isTrue);
        await tester.pumpWidget(const SizedBox.shrink());
        await tester.pump(const Duration(milliseconds: 100));
      },
    );

    testWidgets(
      'creator end publish superseded by rejoin cannot archive new admission',
      (tester) async {
        final now = DateTime.now().millisecondsSinceEpoch ~/ 1000;
        final endPublishGate = Completer<void>();
        final media = Queue<_HuddleTestMedia>.of([
          _HuddleTestMedia(),
          _HuddleTestMedia(),
        ]);
        final transports = Queue<_HuddleTestTransport>.of([
          _HuddleTestTransport(),
          _HuddleTestTransport(),
        ]);
        final relaySession = _ReconnectingRelaySession(
          huddleEndPublishGate: endPublishGate.future,
        );
        String? archivedChannelId;

        await tester.pumpWidget(
          _buildTestable(
            messages: [
              _huddleMsg(
                id: 'stale-creator-end',
                kind: EventKind.huddleStarted,
                pubkey: 'self',
                createdAt: now,
              ),
            ],
            users: const {'self': UserProfile(pubkey: 'self')},
            relayConfigNotifier: _HuddleRelayConfigNotifier(),
            relaySessionNotifier: relaySession,
            huddleCurrentPubkey: 'self',
            huddleMediaFactory: media.removeFirst,
            huddleTransportFactory: (_) => transports.removeFirst(),
            createChannelActions: (ref) => _FakeChannelActions(
              ref,
              onArchiveChannel: (channelId) async =>
                  archivedChannelId = channelId,
            ),
          ),
        );
        await tester.pumpAndSettle();
        await tester.tap(find.widgetWithText(FilledButton, 'Join'));
        await tester.pumpAndSettle();

        final container = ProviderScope.containerOf(
          tester.element(find.byType(MobileHuddleShell)),
        );
        final controller = container.read(
          mobileHuddleControllerProvider.notifier,
        );
        final staleEnd = controller.end();
        await relaySession.huddleEndPublishStarted.future;
        final rejoin = controller.join(
          parentChannelId: _channelId,
          ephemeralChannelId: _huddleChannelId,
          startedBy: 'self',
          startedEventId: 'stale-creator-end',
        );
        endPublishGate.complete();
        await staleEnd;
        await rejoin;
        await tester.pump();

        expect(relaySession.publishedKinds, contains(EventKind.huddleEnded));
        expect(archivedChannelId, isNull);
        await tester.pumpWidget(const SizedBox.shrink());
        await tester.pump(const Duration(milliseconds: 100));
      },
    );

    testWidgets('disables Join after the matching huddle end event', (
      tester,
    ) async {
      final now = DateTime.now().millisecondsSinceEpoch ~/ 1000;
      await tester.pumpWidget(
        _buildTestable(
          messages: [
            _huddleMsg(
              id: 'ended-huddle-start',
              kind: EventKind.huddleStarted,
              createdAt: now,
            ),
            _huddleMsg(
              id: 'ended-huddle-end',
              kind: EventKind.huddleEnded,
              createdAt: now + 1,
            ),
          ],
          users: {
            'alice': const UserProfile(pubkey: 'alice', displayName: 'Alice'),
          },
        ),
      );
      await tester.pumpAndSettle();

      expect(find.text('Huddle ended'), findsOneWidget);
      final ended = tester.widget<FilledButton>(
        find.widgetWithText(FilledButton, 'Ended'),
      );
      expect(ended.onPressed, isNull);
    });

    for (final huddleEvent in [
      (kind: EventKind.huddleStarted, action: 'started a huddle'),
      (kind: EventKind.huddleEnded, action: 'ended the huddle'),
    ]) {
      testWidgets(
        '${huddleEvent.action} aligns its author and body with a regular message',
        (tester) async {
          await tester.pumpWidget(
            _buildTestable(
              messages: [
                _textMsg(
                  id: 'regular-message',
                  pubkey: 'alice',
                  content: 'Regular message',
                  createdAt: 1000,
                ),
                _huddleMsg(
                  id: 'huddle-message',
                  kind: huddleEvent.kind,
                  pubkey: 'bob',
                  createdAt: 1010,
                ),
              ],
              users: {
                'alice': const UserProfile(
                  pubkey: 'alice',
                  displayName: 'Alice',
                ),
                'bob': const UserProfile(pubkey: 'bob', displayName: 'Bob'),
              },
            ),
          );
          await tester.pumpAndSettle();

          final regularRow = find.byKey(
            const ValueKey('message-row-regular-message'),
          );
          final huddleRow = find.byKey(
            const ValueKey('system-message-row-huddle-message'),
          );
          final regularAvatar = tester.getRect(
            find
                .descendant(of: regularRow, matching: find.byType(CircleAvatar))
                .first,
          );
          final huddleAvatar = tester.getRect(
            find
                .descendant(of: huddleRow, matching: find.byType(CircleAvatar))
                .first,
          );
          final regularAuthor = tester.getRect(
            find.byKey(const ValueKey('message-author-regular-message')),
          );
          final huddleAuthor = tester.getRect(
            find.byKey(const ValueKey('system-message-author-bob')),
          );
          final regularBody = tester.getRect(findRichText('Regular message'));
          final huddleBody = tester.getRect(findRichText(huddleEvent.action));

          expect(
            huddleAuthor.top - huddleAvatar.top,
            closeTo(regularAuthor.top - regularAvatar.top, 0.01),
          );
          expect(
            huddleBody.top - huddleAuthor.bottom,
            closeTo(regularBody.top - regularAuthor.bottom, 0.01),
          );
        },
      );
    }

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
                kind: EventKind.huddleEnded,
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

    testWidgets(
      'profile sheet shows the poster before autoplay and restores it on tap',
      (tester) async {
        const posterUrl = 'https://relay.example/media/alice-poster.png';
        const animationUrl = 'https://relay.example/media/alice-avatar.png';
        final profileUrl =
            '$posterUrl#buzz-anim=${Uri.encodeComponent(animationUrl)}';
        final animationResponse = Completer<http.Response>();
        final mediaClient = http_testing.MockClient(
          (request) => request.url.toString() == animationUrl
              ? animationResponse.future
              : Future.value(http.Response.bytes(_transparentPng, 200)),
        );
        addTearDown(mediaClient.close);

        await tester.pumpWidget(
          _buildTestable(
            messages: [
              _huddleMsg(
                id: 'sys-huddle-animated-avatar',
                kind: EventKind.huddleStarted,
                pubkey: 'alice',
              ),
            ],
            users: {
              'alice': UserProfile(
                pubkey: 'alice',
                displayName: 'Alice',
                avatarUrl: profileUrl,
              ),
            },
            mediaClient: mediaClient,
          ),
        );
        await tester.pumpAndSettle();

        await tester.tap(find.byType(CircleAvatar));
        await tester.pumpAndSettle();

        expect(find.byType(UserProfileSheet), findsOneWidget);
        expect(
          find.byKey(const ValueKey('progressive-animated-avatar-poster')),
          findsOneWidget,
        );
        expect(
          find.byKey(
            const ValueKey('progressive-animated-avatar-animation-loading'),
          ),
          findsOneWidget,
        );

        animationResponse.complete(http.Response.bytes(_transparentPng, 200));
        await tester.runAsync(
          () => Future<void>.delayed(const Duration(milliseconds: 50)),
        );
        await tester.pumpAndSettle();

        expect(
          find.byKey(
            const ValueKey('progressive-animated-avatar-animation-ready'),
          ),
          findsOneWidget,
        );
        expect(
          find.byKey(const ValueKey('progressive-animated-avatar-poster')),
          findsNothing,
        );

        await tester.tap(find.byKey(const ValueKey('selected-profile-avatar')));
        await tester.pumpAndSettle();

        expect(
          find.byKey(const ValueKey('progressive-animated-avatar-animation')),
          findsNothing,
        );
        expect(
          tester
              .widget<MediaImage>(
                find.descendant(
                  of: find.byKey(const ValueKey('selected-profile-avatar')),
                  matching: find.byType(MediaImage),
                ),
              )
              .url,
          posterUrl,
        );

        await tester.tap(find.byKey(const ValueKey('selected-profile-avatar')));
        await tester.pumpAndSettle();

        expect(
          find.byKey(const ValueKey('progressive-animated-avatar-animation')),
          findsOneWidget,
        );
        expect(
          find.byKey(const ValueKey('progressive-animated-avatar-poster')),
          findsNothing,
        );
      },
    );

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
    testWidgets('aligns the Channel Details iOS back control with channels', (
      tester,
    ) async {
      debugDefaultTargetPlatformOverride = TargetPlatform.iOS;
      addTearDown(() => debugDefaultTargetPlatformOverride = null);

      await tester.pumpWidget(
        _buildTestable(
          messages: const [],
          home: Builder(
            builder: (context) => Scaffold(
              body: TextButton(
                onPressed: () => Navigator.of(context).push(
                  MaterialPageRoute<void>(
                    builder: (_) => ChannelDetailPage(channel: _testChannel),
                  ),
                ),
                child: const Text('Open channel'),
              ),
            ),
          ),
        ),
      );
      await tester.pumpAndSettle();

      await tester.tap(find.text('Open channel'));
      await tester.pumpAndSettle();

      final channelBack = find.byKey(const ValueKey('channel-ios-glass-back'));
      final channelNativeView = tester.widget<UiKitView>(
        find.descendant(of: channelBack, matching: find.byType(UiKitView)),
      );
      final channelParams =
          channelNativeView.creationParams as Map<String, Object>;
      final channelButtonCenter =
          tester.getTopLeft(channelBack).dx +
          (channelParams['buttonCenterX']! as double);

      await tester.tap(
        find.byKey(const ValueKey('channel-header-settings-trigger')),
      );
      await tester.pumpAndSettle();

      final detailsBack = find.byKey(
        const ValueKey('channel-details-ios-glass-back'),
      );
      final detailsNativeView = tester.widget<UiKitView>(
        find.descendant(of: detailsBack, matching: find.byType(UiKitView)),
      );
      final detailsParams =
          detailsNativeView.creationParams as Map<String, Object>;
      final detailsButtonCenter =
          tester.getTopLeft(detailsBack).dx +
          (detailsParams['buttonCenterX']! as double);
      debugDefaultTargetPlatformOverride = null;

      expect(detailsBack, findsOneWidget);
      expect(
        detailsParams['buttonCenterX'],
        iosGlassChannelHeaderButtonCenterX,
      );
      expect(
        detailsParams['hitTargetWidth'],
        iosGlassChannelHeaderLeadingWidth,
      );
      expect(detailsButtonCenter, moreOrLessEquals(channelButtonCenter));
      expect(
        tester.getRect(detailsBack).width,
        iosGlassChannelHeaderLeadingWidth,
      );
      expect(tester.takeException(), isNull);
    });

    testWidgets('matches the channel header placement on iOS', (tester) async {
      debugDefaultTargetPlatformOverride = TargetPlatform.iOS;
      addTearDown(() => debugDefaultTargetPlatformOverride = null);
      const nativeChannel = MethodChannel('buzz/navigation_glass/43');
      tester.binding.defaultBinaryMessenger.setMockMethodCallHandler(
        nativeChannel,
        (_) async => null,
      );
      addTearDown(
        () => tester.binding.defaultBinaryMessenger.setMockMethodCallHandler(
          nativeChannel,
          null,
        ),
      );

      final root = _textMsg(
        id: 'thread-header-root',
        pubkey: 'alice',
        content: 'Thread root',
      );
      final timelineMessages = formatTimeline([root]);

      await tester.pumpWidget(
        _buildTestable(
          messages: [root],
          textScaler: const TextScaler.linear(2),
          home: Builder(
            builder: (context) => Scaffold(
              body: TextButton(
                onPressed: () => Navigator.of(context).push(
                  MaterialPageRoute<void>(
                    builder: (_) => ThreadDetailPage(
                      threadHead: timelineMessages.single,
                      allMessages: timelineMessages,
                      channelId: _testChannel.id,
                      currentPubkey: null,
                      isMember: true,
                      isArchived: false,
                    ),
                  ),
                ),
                child: const Text('Open thread'),
              ),
            ),
          ),
        ),
      );
      await tester.pumpAndSettle();

      await tester.tap(find.text('Open thread'));
      await tester.pumpAndSettle();

      final backFinder = find.byKey(const ValueKey('thread-ios-glass-back'));
      final nativeView = tester.widget<UiKitView>(
        find.descendant(of: backFinder, matching: find.byType(UiKitView)),
      );
      expect(nativeView.viewType, IosGlassNavigationButton.viewType);
      expect(
        (nativeView.creationParams as Map<String, Object>)['buttonCenterX'],
        iosGlassChannelHeaderButtonCenterX,
      );
      expect(
        (nativeView.creationParams as Map<String, Object>)['hitTargetWidth'],
        iosGlassChannelHeaderLeadingWidth,
      );
      expect(
        (nativeView.creationParams as Map<String, Object>)['hitTargetHeight'],
        48.0,
      );

      final backRect = tester.getRect(backFinder);
      final titleRect = tester.getRect(
        find.byKey(const ValueKey('thread-app-bar-title')),
      );
      expect(backRect.width, iosGlassChannelHeaderLeadingWidth);
      expect(
        titleRect.left - backRect.right,
        moreOrLessEquals(iosGlassChannelHeaderTitleSpacing),
      );
      expect(tester.takeException(), isNull);

      nativeView.onPlatformViewCreated!(43);
      await tester.pump();
      await tester.binding.defaultBinaryMessenger.handlePlatformMessage(
        nativeChannel.name,
        nativeChannel.codec.encodeMethodCall(const MethodCall('pressed')),
        (_) {},
      );
      await tester.pumpAndSettle();

      expect(find.byType(ThreadDetailPage), findsNothing);
      expect(find.text('Open thread'), findsOneWidget);
      debugDefaultTargetPlatformOverride = null;
    });

    testWidgets(
      'keeps the native iOS glass header aligned at large text sizes',
      (tester) async {
        debugDefaultTargetPlatformOverride = TargetPlatform.iOS;
        addTearDown(() => debugDefaultTargetPlatformOverride = null);

        final message = _textMsg(
          id: 'avatar-alignment',
          pubkey: 'alice',
          content: 'Hello',
          createdAt: 1000,
        );

        await tester.pumpWidget(
          _buildTestable(
            messages: [message],
            textScaler: const TextScaler.linear(2),
            users: const {
              'alice': UserProfile(pubkey: 'alice', displayName: 'Alice'),
            },
            home: Builder(
              builder: (context) => Scaffold(
                body: TextButton(
                  onPressed: () => Navigator.of(context).push(
                    MaterialPageRoute<void>(
                      builder: (_) => ChannelDetailPage(channel: _testChannel),
                    ),
                  ),
                  child: const Text('Open channel'),
                ),
              ),
            ),
          ),
        );
        await tester.pumpAndSettle();

        await tester.tap(find.text('Open channel'));
        await tester.pumpAndSettle();

        final nativeViewFinder = find.descendant(
          of: find.byKey(const ValueKey('channel-ios-glass-back')),
          matching: find.byType(UiKitView),
        );
        final nativeView = tester.widget<UiKitView>(nativeViewFinder);
        expect(nativeView.viewType, 'buzz/navigation_glass');
        expect(
          (nativeView.creationParams as Map<String, Object>)['icon'],
          'back',
        );
        expect(
          (nativeView.creationParams as Map<String, Object>)['brightness'],
          'light',
        );
        expect(
          (nativeView.creationParams as Map<String, Object>)['buttonCenterX'],
          38.0,
        );
        final backButtonRect = tester.getRect(
          find.byKey(const ValueKey('channel-ios-glass-back')),
        );
        expect(backButtonRect.width, 58);
        final channelIconRect = tester.getRect(
          find.byKey(const ValueKey('channel-header-avatar')),
        );
        expect(
          channelIconRect.left - backButtonRect.right,
          moreOrLessEquals(Grid.xs),
        );
        expect(
          backButtonRect.center.dy,
          moreOrLessEquals(channelIconRect.center.dy),
        );
        expect(tester.takeException(), isNull);
        debugDefaultTargetPlatformOverride = null;
      },
    );

    for (final platform in [TargetPlatform.android, TargetPlatform.iOS]) {
      testWidgets(
        'keeps a narrow long channel title aligned on ${platform.name}',
        (tester) async {
          final previousPlatform = debugDefaultTargetPlatformOverride;
          debugDefaultTargetPlatformOverride = platform;
          addTearDown(
            () => debugDefaultTargetPlatformOverride = previousPlatform,
          );
          tester.view.physicalSize = const Size(320, 700);
          tester.view.devicePixelRatio = 1;
          addTearDown(tester.view.reset);
          final channel = _testChannel.copyWith(
            name: 'a-very-long-channel-name-that-must-truncate',
          );

          await tester.pumpWidget(
            _buildTestable(
              messages: const [],
              channel: channel,
              home: Builder(
                builder: (context) => Scaffold(
                  body: TextButton(
                    onPressed: () => Navigator.of(context).push(
                      MaterialPageRoute<void>(
                        builder: (_) => ChannelDetailPage(channel: channel),
                      ),
                    ),
                    child: const Text('Open channel'),
                  ),
                ),
              ),
            ),
          );
          await tester.pumpAndSettle();

          await tester.tap(find.text('Open channel'));
          await tester.pumpAndSettle();

          final backRect = platform == TargetPlatform.iOS
              ? tester.getRect(
                  find.byKey(const ValueKey('channel-ios-glass-back')),
                )
              : tester.getRect(find.byTooltip('Back'));
          final avatarRect = tester.getRect(
            find.byKey(const ValueKey('channel-header-avatar')),
          );
          final titleSpacing = avatarRect.left - backRect.right;
          final title = tester.renderObject<RenderParagraph>(
            find.byKey(const ValueKey('channel-header-name')),
          );
          final titleDidExceedMaxLines = title.didExceedMaxLines;
          debugDefaultTargetPlatformOverride = previousPlatform;

          expect(
            titleSpacing,
            moreOrLessEquals(
              platform == TargetPlatform.iOS
                  ? iosGlassChannelHeaderTitleSpacing
                  : 0,
            ),
          );
          expect(titleDidExceedMaxLines, isTrue);
          expect(tester.takeException(), isNull);
        },
      );
    }

    testWidgets('shows a tappable channel name and collective member count', (
      tester,
    ) async {
      await tester.pumpWidget(
        _buildTestable(
          messages: [],
          members: List.generate(
            5,
            (index) => ChannelMember(
              pubkey: 'member-$index',
              role: 'member',
              joinedAt: DateTime(2025),
            ),
          ),
        ),
      );
      await tester.pumpAndSettle();

      expect(find.text('general'), findsOneWidget);
      expect(find.text('5 members'), findsOneWidget);
      // The hash icon appears in the app bar and in the compose bar toolbar.
      expect(find.byIcon(LucideIcons.hash), findsAtLeastNWidgets(1));
      expect(
        tester.getSize(find.byKey(const ValueKey('channel-header-avatar'))),
        const Size.square(40),
      );
      final channelHeaderAvatarRect = tester.getRect(
        find.byKey(const ValueKey('channel-header-avatar')),
      );
      final channelHeaderTextStackRect = tester.getRect(
        find.byKey(const ValueKey('channel-header-text-stack')),
      );
      expect(channelHeaderTextStackRect.height, 40);
      expect(
        channelHeaderTextStackRect.center.dy,
        moreOrLessEquals(channelHeaderAvatarRect.center.dy),
      );
      expect(
        tester
            .widget<Text>(find.byKey(const ValueKey('channel-header-name')))
            .style
            ?.fontSize,
        AppTheme.light().textTheme.titleSmall?.fontSize,
      );
      expect(
        tester
            .widget<Text>(find.byKey(const ValueKey('channel-header-name')))
            .style
            ?.fontWeight,
        FontWeight.w600,
      );
      final channelHeaderAvatar = tester.widget<Container>(
        find.byKey(const ValueKey('channel-header-avatar')),
      );
      expect(
        (channelHeaderAvatar.decoration as BoxDecoration).color,
        AppTheme.light().colorScheme.surface,
      );
      final channelHeaderAvatarBorder =
          (channelHeaderAvatar.decoration as BoxDecoration).border! as Border;
      expect(
        channelHeaderAvatarBorder.top.color,
        AppTheme.light().colorScheme.inverseSurface.withValues(alpha: 0.07),
      );
      expect(channelHeaderAvatarBorder.top.width, 1);
      expect(
        channelHeaderAvatarBorder.top.strokeAlign,
        BorderSide.strokeAlignOutside,
      );
      expect(
        tester
            .widget<Icon>(
              find.descendant(
                of: find.byKey(const ValueKey('channel-header-avatar')),
                matching: find.byIcon(LucideIcons.hash),
              ),
            )
            .color,
        AppTheme.light().colorScheme.primary,
      );
      expect(
        tester.getRect(find.byKey(const ValueKey('channel-header-name'))).left -
            tester
                .getRect(find.byKey(const ValueKey('channel-header-avatar')))
                .right,
        moreOrLessEquals(Grid.twelve),
      );
      expect(
        tester
            .widget<Text>(
              find.byKey(const ValueKey('channel-header-member-count')),
            )
            .style
            ?.fontSize,
        AppTheme.light().textTheme.bodySmall?.fontSize,
      );
      expect(
        tester
            .widget<Text>(
              find.byKey(const ValueKey('channel-header-member-count')),
            )
            .style
            ?.color,
        AppTheme.light().colorScheme.onSurface.withValues(alpha: 0.65),
      );
      expect(find.byTooltip('View members'), findsNothing);
      expect(find.byTooltip('Channel actions'), findsNothing);

      await tester.tap(
        find.byKey(const ValueKey('channel-header-settings-trigger')),
      );
      await tester.pumpAndSettle();

      expect(find.text('Channel settings'), findsNothing);
      expect(
        find.byKey(const ValueKey('channel-details-collapsed-title')),
        findsNothing,
      );
      expect(
        find.byKey(const ValueKey('channel-details-avatar')),
        findsOneWidget,
      );
      expect(
        find.byKey(const ValueKey('channel-details-name')),
        findsOneWidget,
      );
      expect(find.text('General discussion'), findsOneWidget);
      expect(find.text('5 members'), findsOneWidget);
      expect(find.text('Preferences'), findsNothing);
      expect(find.text('Star'), findsOneWidget);
      expect(find.text('Mute'), findsOneWidget);
      expect(find.text('Edit'), findsOneWidget);
      expect(find.text('Actions'), findsNothing);
      expect(find.byTooltip('Back'), findsOneWidget);

      var detailsAppBar = tester.widget<FrostedAppBar>(
        find.byType(FrostedAppBar).last,
      );
      expect(detailsAppBar.frosted, isFalse);
      expect(detailsAppBar.frostedSurfaceOpacity, 0);
      expect(detailsAppBar.frostedBlurSigma, 0);
      expect(detailsAppBar.showBottomDivider, isFalse);
      expect(detailsAppBar.centerTitle, isTrue);

      final descriptionBottom = tester
          .getRect(find.byKey(const ValueKey('channel-details-description')))
          .bottom;
      final firstActionTop = tester
          .getRect(find.byKey(const ValueKey('channel-details-star-action')))
          .top;
      expect(firstActionTop - descriptionBottom, closeTo(Grid.sm, 0.5));
      expect(
        tester
            .getSize(find.byKey(const ValueKey('channel-details-star-action')))
            .height,
        68 + (Grid.xxs * 2),
      );

      final firstActionBottom = tester
          .getRect(find.byKey(const ValueKey('channel-details-star-action')))
          .bottom;
      final membersLabelTop = tester.getRect(find.text('5 members')).top;
      expect(membersLabelTop - firstActionBottom, closeTo(Grid.sm, 0.5));
      expect(
        tester
            .widget<AppListCard>(
              find.byKey(const ValueKey('channel-details-members-card')),
            )
            .verticalPadding,
        Grid.twelve,
      );
      expect(find.text('Channel'), findsNothing);

      await tester.drag(
        find.byKey(const ValueKey('channel-details-page-list')),
        const Offset(0, -300),
      );
      await tester.pumpAndSettle();
      expect(
        find.byKey(const ValueKey('channel-details-collapsed-title')),
        findsOneWidget,
      );
      final collapsedTitle = find.byKey(
        const ValueKey('channel-details-collapsed-title'),
      );
      expect(
        tester.getCenter(collapsedTitle).dx,
        closeTo(tester.getCenter(find.byType(FrostedAppBar).last).dx, 0.5),
      );
      expect(find.text('Channel'), findsNothing);
      detailsAppBar = tester.widget<FrostedAppBar>(
        find.byType(FrostedAppBar).last,
      );
      expect(detailsAppBar.frosted, isTrue);
      expect(detailsAppBar.frostedSurfaceOpacity, 0.5);
      expect(detailsAppBar.frostedBlurSigma, 20);
      expect(detailsAppBar.showBottomDivider, isTrue);
      expect(detailsAppBar.bottomDividerOpacity, 0.07);
      expect(
        tester
            .widget<AppListCard>(
              find.byKey(const ValueKey('channel-details-channel-card')),
            )
            .verticalPadding,
        Grid.twelve,
      );

      await tester.tap(find.byTooltip('Back'));
      await tester.pumpAndSettle();
      expect(
        find.byKey(const ValueKey('channel-header-settings-trigger')),
        findsOneWidget,
      );
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
    testWidgets('fades the target highlight in after the thread route lands', (
      tester,
    ) async {
      final root = _textMsg(
        id: 'root',
        pubkey: 'alice',
        content: 'Thread root',
        createdAt: 1000,
      );
      final target = _textMsg(
        id: 'target',
        pubkey: 'bob',
        content: 'Target reply',
        createdAt: 1100,
        extraTags: const [
          ['e', 'root', '', 'reply'],
        ],
      );
      final timelineMessages = formatTimeline([root, target]);
      final threadHead = timelineMessages.firstWhere(
        (message) => message.id == root.id,
      );

      await tester.pumpWidget(
        _buildTestable(
          messages: [root, target],
          threadReplies: {
            'root': [target],
          },
          users: const {
            'alice': UserProfile(pubkey: 'alice', displayName: 'Alice'),
            'bob': UserProfile(pubkey: 'bob', displayName: 'Bob'),
          },
          home: Builder(
            builder: (context) => Scaffold(
              body: Center(
                child: TextButton(
                  onPressed: () => Navigator.of(context).push(
                    MaterialPageRoute<void>(
                      builder: (_) => ThreadDetailPage(
                        threadHead: threadHead,
                        allMessages: timelineMessages,
                        channelId: _testChannel.id,
                        currentPubkey: null,
                        isMember: true,
                        isArchived: false,
                        initialMessageId: 'target',
                      ),
                    ),
                  ),
                  child: const Text('Open highlighted thread'),
                ),
              ),
            ),
          ),
        ),
      );
      await tester.pumpAndSettle();

      await tester.tap(find.text('Open highlighted thread'));
      await tester.pump();
      await tester.pump(const Duration(milliseconds: 1));

      final threadRoute =
          ModalRoute.of(tester.element(find.byType(ThreadDetailPage)))!
              as MaterialPageRoute<void>;
      expect(threadRoute.animation!.status, AnimationStatus.forward);
      final transitionDecoration =
          tester
                  .widget<DecoratedBox>(
                    find.byKey(const ValueKey('thread-message-target')),
                  )
                  .decoration
              as BoxDecoration;
      expect(transitionDecoration.color, Colors.transparent);

      await tester.pump(threadRoute.transitionDuration);
      expect(threadRoute.animation!.status, AnimationStatus.completed);
      await tester.pump();
      await tester.pump();
      final landedDecoration =
          tester
                  .widget<DecoratedBox>(
                    find.byKey(const ValueKey('thread-message-target')),
                  )
                  .decoration
              as BoxDecoration;
      expect(landedDecoration.color, Colors.transparent);

      await tester.pump(const Duration(milliseconds: 50));
      await tester.pump(const Duration(milliseconds: 150));
      final enteringDecoration =
          tester
                  .widget<DecoratedBox>(
                    find.byKey(const ValueKey('thread-message-target')),
                  )
                  .decoration
              as BoxDecoration;
      expect(enteringDecoration.color!.a, greaterThan(0));
      expect(enteringDecoration.color!.a, lessThan(0.12));

      await tester.pump(const Duration(milliseconds: 150));
      final visibleDecoration =
          tester
                  .widget<DecoratedBox>(
                    find.byKey(const ValueKey('thread-message-target')),
                  )
                  .decoration
              as BoxDecoration;
      expect(visibleDecoration.color!.a, closeTo(0.12, 0.001));
    });

    testWidgets('waits for a delayed target jump before highlighting', (
      tester,
    ) async {
      final root = _textMsg(
        id: 'root',
        pubkey: 'alice',
        content: 'Thread root',
        createdAt: 1000,
      );
      final replies = [
        for (var i = 0; i < 40; i++)
          _textMsg(
            id: 'reply-$i',
            pubkey: 'bob',
            content: 'Reply $i',
            createdAt: 1100 + i,
            extraTags: const [
              ['e', 'root', '', 'reply'],
            ],
          ),
      ];
      final timelineMessages = formatTimeline([root, ...replies]);
      final threadHead = timelineMessages.first;
      final replyCompleter = Completer<List<NostrEvent>>();

      await tester.pumpWidget(
        _buildTestable(
          messages: [root],
          pendingThreadReplies: {'root': replyCompleter.future},
          users: const {
            'alice': UserProfile(pubkey: 'alice', displayName: 'Alice'),
            'bob': UserProfile(pubkey: 'bob', displayName: 'Bob'),
          },
          home: Builder(
            builder: (context) => Scaffold(
              body: Center(
                child: TextButton(
                  onPressed: () => Navigator.of(context).push(
                    MaterialPageRoute<void>(
                      builder: (_) => ThreadDetailPage(
                        threadHead: threadHead,
                        allMessages: timelineMessages,
                        channelId: _testChannel.id,
                        currentPubkey: null,
                        isMember: true,
                        isArchived: false,
                        initialMessageId: 'reply-30',
                      ),
                    ),
                  ),
                  child: const Text('Open delayed thread'),
                ),
              ),
            ),
          ),
        ),
      );
      await tester.pumpAndSettle();

      await tester.tap(find.text('Open delayed thread'));
      await tester.pumpAndSettle();
      await tester.pump(const Duration(seconds: 4));

      expect(
        find.byKey(const ValueKey('thread-message-group-reply-30')),
        findsNothing,
      );

      replyCompleter.complete(replies);
      // Flush hydration, target placement, and the paint gate without
      // advancing the 50 ms highlight delay through the Latest control's own
      // entrance animation.
      for (var frame = 0; frame < 8; frame += 1) {
        await tester.pump();
      }

      final target = find.byKey(const ValueKey('thread-message-reply-30'));
      expect(target, findsOneWidget);
      final landedDecoration =
          tester.widget<DecoratedBox>(target).decoration as BoxDecoration;
      expect(landedDecoration.color, Colors.transparent);

      await tester.pump(const Duration(milliseconds: 50));
      await tester.pump(const Duration(milliseconds: 150));
      final enteringDecoration =
          tester.widget<DecoratedBox>(target).decoration as BoxDecoration;
      expect(enteringDecoration.color!.a, greaterThan(0));
      expect(enteringDecoration.color!.a, lessThan(0.12));
    });

    testWidgets('waits for a retry before jumping to a hydrated target', (
      tester,
    ) async {
      final root = _textMsg(
        id: 'root',
        pubkey: 'alice',
        content: 'Thread root',
        createdAt: 1000,
      );
      final target = _textMsg(
        id: 'target',
        pubkey: 'bob',
        content: 'Hydrated target',
        createdAt: 1400,
        extraTags: const [
          ['e', 'root', '', 'reply'],
        ],
      );
      final earlierReplies = [
        for (var i = 0; i < 30; i++)
          _textMsg(
            id: 'reply-$i',
            pubkey: 'bob',
            content: 'Reply $i',
            createdAt: 1100 + i,
            extraTags: const [
              ['e', 'root', '', 'reply'],
            ],
          ),
      ];
      final timelineMessages = formatTimeline([root, target]);
      final firstAttempt = Completer<List<NostrEvent>>();
      var attempts = 0;

      await tester.pumpWidget(
        _buildTestable(
          messages: [root, target],
          providerRetry: (retryCount, _) =>
              retryCount == 0 ? const Duration(seconds: 30) : null,
          localThreadReplies: {
            'root': [target],
          },
          threadReplyLoaders: {
            'root': () {
              attempts++;
              if (attempts == 1) return firstAttempt.future;
              return Future.value([...earlierReplies, target]);
            },
          },
          users: const {
            'alice': UserProfile(pubkey: 'alice', displayName: 'Alice'),
            'bob': UserProfile(pubkey: 'bob', displayName: 'Bob'),
          },
          home: ThreadDetailPage(
            threadHead: timelineMessages.first,
            allMessages: timelineMessages,
            channelId: _testChannel.id,
            currentPubkey: null,
            isMember: true,
            isArchived: false,
            initialMessageId: 'target',
          ),
        ),
      );
      await tester.pump();
      firstAttempt.completeError(Exception('transient thread query failure'));
      await tester.pump();
      await tester.pump();

      final targetFinder = find.byKey(const ValueKey('thread-message-target'));
      expect(targetFinder, findsOneWidget);
      final retryingDecoration =
          tester.widget<DecoratedBox>(targetFinder).decoration as BoxDecoration;
      expect(retryingDecoration.color, Colors.transparent);
      expect(attempts, 1);

      await tester.pump(const Duration(milliseconds: 50));
      await tester.pump(const Duration(milliseconds: 150));
      final stillRetryingDecoration =
          tester.widget<DecoratedBox>(targetFinder).decoration as BoxDecoration;
      expect(stillRetryingDecoration.color, Colors.transparent);

      await tester.pump(const Duration(milliseconds: 2800));
      expect(attempts, 1);
      final expiredJumpDecoration =
          tester.widget<DecoratedBox>(targetFinder).decoration as BoxDecoration;
      expect(expiredJumpDecoration.color, Colors.transparent);

      await tester.pump(const Duration(seconds: 30));
      // Settle the retry's microtasks without advancing the shared Latest
      // control's entrance animation past the highlight's 50 ms delay.
      for (var frame = 0; frame < 8; frame += 1) {
        await tester.pump();
      }

      expect(attempts, 2);
      expect(
        find.byKey(const ValueKey('thread-message-group-target')),
        findsOneWidget,
      );
      final landedDecoration =
          tester.widget<DecoratedBox>(targetFinder).decoration as BoxDecoration;
      expect(landedDecoration.color, Colors.transparent);

      await tester.pump(const Duration(milliseconds: 50));
      await tester.pump(const Duration(milliseconds: 150));
      final highlightedDecoration =
          tester.widget<DecoratedBox>(targetFinder).decoration as BoxDecoration;
      expect(highlightedDecoration.color!.a, greaterThan(0));
    });

    testWidgets('highlights a hydrated target after the thread query fails', (
      tester,
    ) async {
      final root = _textMsg(
        id: 'root',
        pubkey: 'alice',
        content: 'Thread root',
        createdAt: 1000,
      );
      final target = _textMsg(
        id: 'target',
        pubkey: 'bob',
        content: 'Hydrated target',
        createdAt: 1100,
        extraTags: const [
          ['e', 'root', '', 'reply'],
        ],
      );
      final timelineMessages = formatTimeline([root, target]);
      final replyCompleter = Completer<List<NostrEvent>>();

      await tester.pumpWidget(
        _buildTestable(
          messages: [root, target],
          pendingThreadReplies: {'root': replyCompleter.future},
          disableRetries: true,
          users: const {
            'alice': UserProfile(pubkey: 'alice', displayName: 'Alice'),
            'bob': UserProfile(pubkey: 'bob', displayName: 'Bob'),
          },
          home: ThreadDetailPage(
            threadHead: timelineMessages.first,
            allMessages: timelineMessages,
            channelId: _testChannel.id,
            currentPubkey: null,
            isMember: true,
            isArchived: false,
            initialMessageId: 'target',
          ),
        ),
      );
      await tester.pumpAndSettle();

      final targetFinder = find.byKey(const ValueKey('thread-message-target'));
      expect(targetFinder, findsOneWidget);
      final loadingDecoration =
          tester.widget<DecoratedBox>(targetFinder).decoration as BoxDecoration;
      expect(loadingDecoration.color, Colors.transparent);

      replyCompleter.completeError(Exception('thread query failed'));
      for (var i = 0; i < 8; i++) {
        await tester.pump();
      }
      await tester.pump(const Duration(milliseconds: 50));
      await tester.pump(const Duration(milliseconds: 150));

      final highlightedDecoration =
          tester.widget<DecoratedBox>(targetFinder).decoration as BoxDecoration;
      expect(highlightedDecoration.color!.a, greaterThan(0));
      expect(highlightedDecoration.color!.a, lessThan(0.12));
    });

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
      final initialHighlight = decoration.color!;
      expect(initialHighlight, isNot(Colors.transparent));
      expect(initialHighlight.a, closeTo(0.12, 0.001));

      await tester.pump(const Duration(milliseconds: 2999));
      final heldDecoration =
          tester
                  .widget<DecoratedBox>(
                    find.byKey(const ValueKey('thread-message-target')),
                  )
                  .decoration
              as BoxDecoration;
      expect(heldDecoration.color, initialHighlight);

      await tester.pump(const Duration(milliseconds: 1));
      await tester.pump(const Duration(milliseconds: 150));

      final fadingDecoration =
          tester
                  .widget<DecoratedBox>(
                    find.byKey(const ValueKey('thread-message-target')),
                  )
                  .decoration
              as BoxDecoration;
      expect(fadingDecoration.color!.a, greaterThan(0));
      expect(fadingDecoration.color!.a, lessThan(initialHighlight.a));

      await tester.pump(const Duration(milliseconds: 150));
      final dismissedDecoration =
          tester
                  .widget<DecoratedBox>(
                    find.byKey(const ValueKey('thread-message-target')),
                  )
                  .decoration
              as BoxDecoration;
      expect(dismissedDecoration.color, Colors.transparent);
    });

    testWidgets('does not replace a newer route after delayed hydration', (
      tester,
    ) async {
      final root = _textMsg(
        id: 'root',
        pubkey: 'alice',
        content: 'Thread root',
      );
      final messagesNotifier = _FakeMessagesNotifier(const []);

      await tester.pumpWidget(
        _buildTestable(
          messages: const [],
          messagesNotifier: messagesNotifier,
          initialThreadRootId: 'root',
          initialThreadRouteBehavior:
              InitialThreadRouteBehavior.replaceCurrentRoute,
        ),
      );
      await tester.pumpAndSettle();

      final navigator = Navigator.of(
        tester.element(find.byType(ChannelDetailPage)),
      );
      messagesNotifier.setMessages([root]);
      navigator.push(
        MaterialPageRoute<void>(
          builder: (_) => const Scaffold(body: Text('New destination')),
        ),
      );
      await tester.pumpAndSettle();

      expect(find.text('New destination'), findsOneWidget);
      expect(find.byType(ThreadDetailPage), findsNothing);
    });

    testWidgets('replaces a temporary channel route for an initial thread', (
      tester,
    ) async {
      final root = _textMsg(
        id: 'root',
        pubkey: 'alice',
        content: 'Thread root',
      );
      final target = _textMsg(
        id: 'target',
        pubkey: 'bob',
        content: 'Target reply',
        createdAt: 1100,
        extraTags: const [
          ['e', 'root', '', 'reply'],
        ],
      );
      final relaySession = _TrackingRelaySession();

      await tester.pumpWidget(
        _buildTestable(
          messages: [root, target],
          relaySessionNotifier: relaySession,
          threadReplies: {
            'root': [target],
          },
          users: const {
            'alice': UserProfile(pubkey: 'alice', displayName: 'Alice'),
            'bob': UserProfile(pubkey: 'bob', displayName: 'Bob'),
          },
          home: Builder(
            builder: (context) => Scaffold(
              body: Center(
                child: TextButton(
                  onPressed: () => Navigator.of(context).push(
                    MaterialPageRoute<void>(
                      builder: (_) => ChannelDetailPage(
                        channel: _testChannel,
                        initialMessageId: 'target',
                        initialThreadRootId: 'root',
                        initialThreadRouteBehavior:
                            InitialThreadRouteBehavior.replaceCurrentRoute,
                      ),
                    ),
                  ),
                  child: const Text('Open activity thread'),
                ),
              ),
            ),
          ),
        ),
      );
      await tester.pumpAndSettle();

      await tester.tap(find.text('Open activity thread'));
      await tester.pumpAndSettle();

      expect(find.byType(ThreadDetailPage), findsOneWidget);
      expect(find.byType(ChannelDetailPage), findsNothing);
      expect(relaySession.visibleChannels, [_testChannel.id]);

      await tester.pageBack();
      await tester.pumpAndSettle();

      expect(find.text('Open activity thread'), findsOneWidget);
      expect(find.byType(ChannelDetailPage), findsNothing);
      expect(relaySession.visibleChannels, isEmpty);
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

      await tester.tap(find.text('random'));
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
      await tester.tap(find.text('random'));
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

      await tester.tap(find.text('random').last);
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
      expect(
        find.descendant(
          of: find.byType(DayDivider),
          matching: find.text(formatDayHeading(rootCreatedAt)),
        ),
        findsOneWidget,
      );
      expect(
        find.descendant(
          of: find.byType(DayDivider),
          matching: find.text(formatDayHeading(nextDayCreatedAt)),
        ),
        findsOneWidget,
      );
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
      final threadTimestamp = tester.widget<Text>(
        find.byKey(const ValueKey('thread-message-timestamp-thread-root')),
      );
      expect(
        threadTimestamp.style?.fontSize,
        messageTimestampTextStyle.fontSize,
      );
      expect(
        find.descendant(
          of: find.byKey(const ValueKey('thread-message-row-thread-root')),
          matching: find.text('·'),
        ),
        findsNothing,
      );
    });

    testWidgets('thread pins and updates the active date while scrolling', (
      tester,
    ) async {
      tester.view.physicalSize = const Size(400, 800);
      tester.view.devicePixelRatio = 1;
      addTearDown(tester.view.resetPhysicalSize);
      addTearDown(tester.view.resetDevicePixelRatio);

      int timestampForDay(int day, int minute) =>
          DateTime(2025, 1, day, 12, minute).toUtc().millisecondsSinceEpoch ~/
          1000;
      final rootEvent = _textMsg(
        id: 'sticky-thread-root',
        pubkey: 'alice',
        content: 'Thread root',
        createdAt: timestampForDay(1, 0),
      );
      final replies = [
        for (var i = 0; i < 90; i++)
          _textMsg(
            id: 'sticky-reply-$i',
            pubkey: 'bob',
            content: 'Reply $i',
            createdAt: timestampForDay(1 + (i ~/ 30), i % 30),
            extraTags: const [
              ['e', 'sticky-thread-root', '', 'reply'],
            ],
          ),
      ];

      await tester.pumpWidget(
        _buildTestable(
          messages: [rootEvent],
          threadReplies: {'sticky-thread-root': replies},
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

      final listFinder = find.byKey(const ValueKey('thread-message-list'));
      final list = tester.widget<ScrollablePositionedList>(listFinder);
      list.itemScrollController!.jumpTo(index: 40);
      await tester.pumpAndSettle();

      final stickyHeader = find.byKey(
        const ValueKey('thread-sticky-date-header'),
      );
      expect(
        find.descendant(
          of: stickyHeader,
          matching: find.text(formatDayHeading(timestampForDay(2, 0))),
        ),
        findsOneWidget,
      );
      expect(
        find.descendant(
          of: stickyHeader,
          matching: find.byType(BackdropFilter),
        ),
        findsOneWidget,
      );

      list.itemScrollController!.jumpTo(index: 70);
      await tester.pumpAndSettle();
      expect(
        find.descendant(
          of: stickyHeader,
          matching: find.text(formatDayHeading(timestampForDay(3, 0))),
        ),
        findsOneWidget,
      );
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
      await tester.pump();
      await tester.pump(androidImeMetricsSettleDelay);
      await tester.pump();

      expect(
        tester.getBottomLeft(latestReply).dy,
        lessThanOrEqualTo(tester.getTopLeft(composerSurface).dy),
      );
      expect(
        tester
            .state<ScrollableState>(
              find
                  .descendant(
                    of: find.byKey(const ValueKey('thread-message-list')),
                    matching: find.byType(Scrollable),
                  )
                  .first,
            )
            .position
            .isScrollingNotifier
            .value,
        isFalse,
        reason: 'Keyboard layout correction must not start a scroll animation.',
      );
    });

    testWidgets('short thread keeps its head stable when the keyboard opens', (
      tester,
    ) async {
      tester.view.physicalSize = const Size(400, 800);
      tester.view.devicePixelRatio = 1;
      addTearDown(tester.view.reset);

      final rootEvent = _textMsg(
        id: 'thread-root',
        pubkey: 'alice',
        content: 'A short thread',
        createdAt: 1000,
      );

      await tester.pumpWidget(
        _buildTestable(
          messages: [rootEvent],
          threadReplies: const {'thread-root': []},
          users: const {
            'alice': UserProfile(pubkey: 'alice', displayName: 'Alice'),
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

      final head = find.byKey(
        const ValueKey('thread-message-group-thread-root'),
      );
      final initialHeadY = tester.getTopLeft(head).dy;
      expect(initialHeadY, lessThan(300));

      await tester.tap(find.text('Reply in thread…').hitTestable());
      await tester.pumpAndSettle();
      tester.view.viewInsets = const FakeViewPadding(bottom: 300);
      await tester.pump();
      await tester.pump(androidImeMetricsSettleDelay);
      await tester.pump();

      expect(
        tester.getTopLeft(head).dy,
        closeTo(initialHeadY, 1),
        reason: 'A fully visible short thread should remain head-anchored.',
      );
    });

    testWidgets(
      'iOS thread keeps Latest hidden through composer and keyboard frames',
      (tester) async {
        final previousPlatform = debugDefaultTargetPlatformOverride;
        debugDefaultTargetPlatformOverride = TargetPlatform.iOS;
        tester.view.physicalSize = const Size(400, 800);
        tester.view.devicePixelRatio = 1;
        addTearDown(tester.view.reset);
        try {
          final rootEvent = _textMsg(
            id: 'thread-root',
            pubkey: 'alice',
            content: 'A short thread',
            createdAt: 1000,
          );
          final replies = [
            for (var i = 0; i < 6; i++)
              _textMsg(
                id: 'reply-$i',
                pubkey: i.isEven ? 'alice' : 'bob',
                content: i.isEven ? 'hello' : 'testing',
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
              ),
            ),
          );
          await tester.pumpAndSettle();

          expect(
            find.byKey(const ValueKey('thread-jump-to-latest')),
            findsNothing,
          );

          await tester.tap(find.text('Reply in thread…').hitTestable());
          for (var frame = 0; frame < 15; frame += 1) {
            await tester.pump(const Duration(milliseconds: 16));
            expect(
              find.byKey(const ValueKey('thread-jump-to-latest')),
              findsNothing,
              reason:
                  'Composer expansion must not expose Latest while followed '
                  'tail geometry catches up.',
            );
          }

          for (final inset in const [80.0, 160.0, 240.0, 300.0]) {
            tester.view.viewInsets = FakeViewPadding(bottom: inset);
            await tester.pump(const Duration(milliseconds: 16));
            expect(
              find.byKey(const ValueKey('thread-jump-to-latest')),
              findsNothing,
              reason:
                  'IME inset frames must not expose Latest while the composer '
                  'is following the thread tail.',
            );
          }
          await tester.pumpAndSettle();
        } finally {
          debugDefaultTargetPlatformOverride = previousPlatform;
        }
      },
    );

    for (final replyCount in [0, 1]) {
      testWidgets(
        'cached writable $replyCount-reply thread defers dock correction until measured',
        (tester) async {
          tester.view.physicalSize = const Size(400, 800);
          tester.view.devicePixelRatio = 1;
          addTearDown(tester.view.reset);
          final rootEvent = _textMsg(
            id: 'thread-root',
            pubkey: 'alice',
            content: 'Short root',
            createdAt: 1000,
          );
          final replies = [
            if (replyCount == 1)
              _textMsg(
                id: 'reply-0',
                pubkey: 'bob',
                content: 'Short reply',
                createdAt: 1100,
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
          final routeMessages = formatTimeline([rootEvent, ...replies]);
          Navigator.of(tester.element(find.byType(ChannelDetailPage))).push(
            MaterialPageRoute<void>(
              builder: (_) => ThreadDetailPage(
                threadHead: routeMessages.first,
                allMessages: routeMessages,
                channelId: _channelId,
                currentPubkey: 'self',
                isMember: true,
                isArchived: false,
              ),
            ),
          );

          await tester.pump();
          expect(tester.takeException(), isNull);
          final earlyDock = tester.widget<ComposerDockSizeReporter>(
            find.byType(ComposerDockSizeReporter).last,
          );

          await tester.pumpAndSettle();
          expect(tester.takeException(), isNull);
          earlyDock.onHeightChanged(200);
          await tester.pumpAndSettle();
          expect(tester.takeException(), isNull);
          final composerSurface = find.byKey(
            const ValueKey('composer-surface'),
          );
          final tail = find.byKey(
            ValueKey(
              replyCount == 0
                  ? 'thread-message-group-thread-root'
                  : 'thread-message-group-reply-0',
            ),
          );
          expect(tail, findsOneWidget);
          expect(
            tester.getTopLeft(tail).dy,
            greaterThanOrEqualTo(frostedAppBarHeight(tester.element(tail))),
          );
          expect(
            tester.getBottomLeft(tail).dy,
            lessThanOrEqualTo(tester.getTopLeft(composerSurface).dy),
          );
        },
      );
    }

    testWidgets(
      'cached initial thread settles between the app bar and measured composer',
      (tester) async {
        tester.view.physicalSize = const Size(400, 800);
        tester.view.devicePixelRatio = 1;
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
              content: i == 29
                  ? List.filled(33, 'Tall latest reply').join('\n')
                  : 'Reply $i',
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
            ),
          ),
        );
        await tester.pumpAndSettle();

        final latestReply = find.byKey(
          const ValueKey('thread-message-group-reply-29'),
        );
        final latestRect = tester.getRect(latestReply);
        final context = tester.element(latestReply);
        final composerTop = tester
            .getTopLeft(find.byKey(const ValueKey('composer-surface')))
            .dy;
        expect(
          latestRect.top,
          greaterThanOrEqualTo(frostedAppBarHeight(context)),
        );
        expect(latestRect.bottom, lessThanOrEqualTo(composerTop));
      },
    );

    testWidgets('read-only cached thread still settles on its tail', (
      tester,
    ) async {
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
            isMember: false,
            isArchived: false,
          ),
        ),
      );
      await tester.pumpAndSettle();

      final latestReply = find.byKey(
        const ValueKey('thread-message-group-reply-29'),
      );
      expect(latestReply, findsOneWidget);
      expect(
        tester.getTopLeft(latestReply).dy,
        greaterThanOrEqualTo(frostedAppBarHeight(tester.element(latestReply))),
      );
    });

    testWidgets(
      'user drag abandons pending hydration settle until returning to tail',
      (tester) async {
        tester.view.physicalSize = const Size(400, 800);
        tester.view.devicePixelRatio = 1;
        addTearDown(tester.view.reset);

        final rootEvent = _textMsg(
          id: 'thread-root',
          pubkey: 'alice',
          content: 'Thread root',
          createdAt: 1000,
        );
        final replies = [
          for (var i = 0; i < 35; i++)
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

        final provisional = formatTimeline([rootEvent, ...replies.take(30)]);
        Navigator.of(tester.element(find.byType(ChannelDetailPage))).push(
          MaterialPageRoute<void>(
            builder: (_) => ThreadDetailPage(
              threadHead: provisional.first,
              allMessages: provisional,
              channelId: _channelId,
              currentPubkey: 'self',
              isMember: true,
              isArchived: false,
            ),
          ),
        );
        await tester.pumpAndSettle();

        final list = find.byKey(const ValueKey('thread-message-list'));
        await tester.drag(list, const Offset(0, -100));
        await tester.pumpAndSettle();
        final anchor = find.byKey(
          const ValueKey('thread-message-group-reply-10'),
        );
        final anchorTop = tester.getTopLeft(anchor).dy;

        completer.complete(replies);
        await tester.pumpAndSettle();

        expect(anchor, findsOneWidget);
        expect(tester.getTopLeft(anchor).dy, closeTo(anchorTop, 0.5));
        expect(
          find.byKey(const ValueKey('thread-message-group-reply-34')),
          findsNothing,
        );

        for (var i = 0; i < 12; i++) {
          await tester.drag(list, const Offset(0, -100));
          await tester.pumpAndSettle();
        }
        final latestReply = find.byKey(
          const ValueKey('thread-message-group-reply-34'),
        );
        expect(latestReply, findsOneWidget);

        tester.view.viewInsets = const FakeViewPadding(bottom: 300);
        await tester.pumpAndSettle();

        expect(
          tester.getBottomLeft(latestReply).dy,
          lessThanOrEqualTo(
            tester
                .getTopLeft(find.byKey(const ValueKey('composer-surface')))
                .dy,
          ),
        );
      },
    );

    testWidgets('active drag cancels a queued hydrated deep-link jump', (
      tester,
    ) async {
      final rootEvent = _textMsg(
        id: 'thread-root',
        pubkey: 'alice',
        content: 'Thread root',
        createdAt: 1000,
      );
      final replies = [
        for (var i = 0; i < 35; i++)
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

      final provisional = formatTimeline([rootEvent, ...replies.take(30)]);
      Navigator.of(tester.element(find.byType(ChannelDetailPage))).push(
        MaterialPageRoute<void>(
          builder: (_) => ThreadDetailPage(
            threadHead: provisional.first,
            allMessages: provisional,
            channelId: _channelId,
            currentPubkey: 'self',
            isMember: true,
            isArchived: false,
            initialMessageId: 'reply-5',
          ),
        ),
      );
      await tester.pumpAndSettle();

      final anchor = find.byKey(
        const ValueKey('thread-message-group-thread-root'),
      );
      final anchorTop = tester.getTopLeft(anchor).dy;
      completer.complete(replies);
      await tester.pump();

      // Authoritative hydration has queued the deep-link jump. A primary drag
      // takes ownership before the shared deferred-intent boundary executes it.
      tester
          .widget<KeyboardDismissOnDrag>(find.byType(KeyboardDismissOnDrag))
          .onUserScrollStart!();
      await tester.pumpAndSettle();

      expect(anchor, findsOneWidget);
      expect(tester.getTopLeft(anchor).dy, closeTo(anchorTop, 0.5));
    });

    testWidgets('user drag invalidates an already queued hydration settle', (
      tester,
    ) async {
      final rootEvent = _textMsg(
        id: 'thread-root',
        pubkey: 'alice',
        content: 'Thread root',
        createdAt: 1000,
      );
      final replies = [
        for (var i = 0; i < 35; i++)
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

      final provisional = formatTimeline([rootEvent, ...replies.take(30)]);
      Navigator.of(tester.element(find.byType(ChannelDetailPage))).push(
        MaterialPageRoute<void>(
          builder: (_) => ThreadDetailPage(
            threadHead: provisional.first,
            allMessages: provisional,
            channelId: _channelId,
            currentPubkey: 'self',
            isMember: true,
            isArchived: false,
          ),
        ),
      );
      await tester.pumpAndSettle();

      final anchor = find.byKey(
        const ValueKey('thread-message-group-thread-root'),
      );
      final anchorTop = tester.getTopLeft(anchor).dy;
      completer.complete(replies);
      await tester.pump();

      // Hydration has scheduled the settle's second post-frame callback. A
      // drag starts before another frame can run it.
      tester
          .widget<KeyboardDismissOnDrag>(find.byType(KeyboardDismissOnDrag))
          .onUserScrollStart!();
      await tester.pumpAndSettle();

      expect(anchor, findsOneWidget);
      expect(tester.getTopLeft(anchor).dy, closeTo(anchorTop, 0.5));
      expect(
        find.byKey(const ValueKey('thread-message-group-reply-34')),
        findsNothing,
      );
    });

    testWidgets(
      'keyboard-open initial hydration settles within the list viewport',
      (tester) async {
        tester.view.physicalSize = const Size(400, 800);
        tester.view.devicePixelRatio = 1;
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
              content: i == 29
                  ? List.filled(15, 'Tall latest reply').join('\n')
                  : 'Reply $i',
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

        await tester.tap(find.text('Reply in thread…').hitTestable());
        await tester.pumpAndSettle();
        tester.view.viewInsets = const FakeViewPadding(bottom: 300);
        await tester.pumpAndSettle();

        final list = find.byKey(const ValueKey('thread-message-list'));
        final listHeight = tester.getSize(list).height;
        final mediaQueryHeight = MediaQuery.sizeOf(tester.element(list)).height;
        expect(
          listHeight,
          closeTo(mediaQueryHeight, 0.5),
          reason: 'Android keeps the thread viewport fixed behind the IME.',
        );

        completer.complete(replies);
        await tester.pumpAndSettle();

        final latestReply = find.byKey(
          const ValueKey('thread-message-group-reply-29'),
        );
        final latestRect = tester.getRect(latestReply);
        final context = tester.element(latestReply);
        final composerTop = tester
            .getTopLeft(find.byKey(const ValueKey('composer-surface')))
            .dy;
        expect(
          latestRect.top,
          greaterThanOrEqualTo(frostedAppBarHeight(context)),
        );
        expect(latestRect.bottom, lessThanOrEqualTo(composerTop));
      },
    );

    testWidgets('short initial thread hydration remains top-anchored', (
      tester,
    ) async {
      final previousPlatform = debugDefaultTargetPlatformOverride;
      debugDefaultTargetPlatformOverride = TargetPlatform.iOS;
      try {
        final rootEvent = _textMsg(
          id: 'thread-root',
          pubkey: 'alice',
          content: 'Thread root',
          createdAt: 1000,
        );
        final replies = [
          _textMsg(
            id: 'reply-1',
            pubkey: 'bob',
            content: 'First reply',
            createdAt: 1100,
            extraTags: const [
              ['e', 'thread-root', '', 'reply'],
            ],
          ),
          _textMsg(
            id: 'reply-2',
            pubkey: 'bob',
            content: 'Second reply',
            createdAt: 1101,
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

        final headFinder = find.byKey(
          const ValueKey('thread-message-group-thread-root'),
        );
        final initialHeadY = tester.getTopLeft(headFinder).dy;
        const latestButton = ValueKey('thread-jump-to-latest');
        expect(find.byKey(latestButton), findsNothing);

        completer.complete(replies);
        await tester.pump();
        for (var frame = 0; frame < 8; frame++) {
          expect(
            find.byKey(latestButton),
            findsNothing,
            reason:
                'Ordinary thread entry must not expose Latest on frame $frame.',
          );
          await tester.pump();
        }
        await tester.pumpAndSettle();

        expect(headFinder, findsOneWidget);
        expect(
          find.byKey(const ValueKey('thread-message-group-reply-2')),
          findsOneWidget,
        );
        expect(tester.getTopLeft(headFinder).dy, closeTo(initialHeadY, 0.5));
        expect(find.byKey(latestButton), findsNothing);
      } finally {
        debugDefaultTargetPlatformOverride = previousPlatform;
      }
    });

    testWidgets(
      'slow initial hydration requests the frame that settles the latest reply',
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

        completer.complete(replies);
        await tester.pump();
        expect(tester.binding.hasScheduledFrame, isTrue);
        await tester.pumpAndSettle();

        expect(
          find.byKey(const ValueKey('thread-message-group-thread-root')),
          findsNothing,
        );
        expect(
          find.byKey(const ValueKey('thread-message-group-reply-29')),
          findsOneWidget,
        );
      },
    );

    testWidgets(
      'initial thread hydration settles on the latest reply after pagination',
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
        final messagesNotifier = _FakeMessagesNotifier([rootEvent]);

        await tester.pumpWidget(
          _buildTestable(
            messages: [rootEvent],
            messagesNotifier: messagesNotifier,
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
        expect(
          find.byKey(const ValueKey('thread-jump-to-latest')),
          findsNothing,
        );

        completer.complete(replies);
        await tester.pump();
        final latestLiveReply = _textMsg(
          id: 'reply-live',
          pubkey: 'bob',
          content: List.filled(10, 'Tall live reply').join('\n'),
          createdAt: 1200,
          extraTags: const [
            ['e', 'thread-root', '', 'reply'],
          ],
        );
        messagesNotifier.setMessages([rootEvent, latestLiveReply]);
        await tester.pumpAndSettle();

        expect(
          find.byKey(const ValueKey('thread-message-group-thread-root')),
          findsNothing,
        );
        expect(
          find.byKey(const ValueKey('thread-message-group-reply-live')),
          findsOneWidget,
        );
        final listRect = tester.getRect(
          find.byKey(const ValueKey('thread-message-list')),
        );
        final latestReply = find.byKey(
          const ValueKey('thread-message-group-reply-live'),
        );
        final latestRect = tester.getRect(latestReply);
        final composerTop = tester
            .getTopLeft(find.byKey(const ValueKey('composer-surface')))
            .dy;
        expect(latestRect.bottom, lessThanOrEqualTo(composerTop));
        expect(latestRect.bottom, greaterThan(listRect.center.dy));
      },
    );

    testWidgets(
      'active primary drag rejects queued remote and local reply tail work',
      (tester) async {
        final rootEvent = _textMsg(
          id: 'thread-root',
          pubkey: 'alice',
          content: 'Root',
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
        final messagesNotifier = _FakeMessagesNotifier([rootEvent]);
        await tester.pumpWidget(
          _buildTestable(
            messages: [rootEvent],
            messagesNotifier: messagesNotifier,
            threadReplies: {'thread-root': replies},
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
        final lifecycle = tester.widget<KeyboardDismissOnDrag>(
          find.byType(KeyboardDismissOnDrag),
        );
        lifecycle.onUserScrollStart!();
        final anchor = find.byKey(
          const ValueKey('thread-message-group-reply-20'),
        );
        final anchorTop = tester.getTopLeft(anchor).dy;
        final remoteReply = _textMsg(
          id: 'reply-remote',
          pubkey: 'bob',
          content: 'Remote',
          createdAt: 1200,
          extraTags: const [
            ['e', 'thread-root', '', 'reply'],
          ],
        );
        messagesNotifier.setMessages([rootEvent, remoteReply]);
        await tester.pumpAndSettle();
        expect(tester.getTopLeft(anchor).dy, closeTo(anchorTop, 0.5));
        final localReply = _textMsg(
          id: 'reply-local',
          pubkey: 'self',
          content: 'Local',
          createdAt: 1201,
          extraTags: const [
            ['e', 'thread-root', '', 'reply'],
          ],
        );
        messagesNotifier.setMessages([rootEvent, remoteReply, localReply]);
        await tester.pumpAndSettle();
        expect(tester.getTopLeft(anchor).dy, closeTo(anchorTop, 0.5));
        lifecycle.onUserScrollEnd!();
      },
    );

    testWidgets('idle detached local reply retains force-visible behavior', (
      tester,
    ) async {
      final rootEvent = _textMsg(
        id: 'thread-root',
        pubkey: 'alice',
        content: 'Root',
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
      final messagesNotifier = _FakeMessagesNotifier([rootEvent]);
      await tester.pumpWidget(
        _buildTestable(
          messages: [rootEvent],
          messagesNotifier: messagesNotifier,
          threadReplies: {'thread-root': replies},
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
      final list = find.byKey(const ValueKey('thread-message-list'));
      for (var i = 0; i < 4; i++) {
        await tester.drag(list, const Offset(0, 100));
        await tester.pumpAndSettle();
      }
      final localReply = _textMsg(
        id: 'reply-local',
        pubkey: 'self',
        content: 'Local',
        createdAt: 1200,
        extraTags: const [
          ['e', 'thread-root', '', 'reply'],
        ],
      );
      messagesNotifier.setMessages([rootEvent, localReply]);
      await tester.pumpAndSettle();
      expect(
        find.byKey(const ValueKey('thread-message-group-reply-local')),
        findsOneWidget,
      );

      final laterRemoteReplies = [
        for (var i = 0; i < 10; i++)
          _textMsg(
            id: 'reply-after-local-$i',
            pubkey: 'bob',
            content: List.filled(8, 'Tall remote reply $i').join('\n'),
            createdAt: 1300 + i,
            extraTags: const [
              ['e', 'thread-root', '', 'reply'],
            ],
          ),
      ];
      messagesNotifier.setMessages([
        rootEvent,
        localReply,
        ...laterRemoteReplies,
      ]);
      await tester.pumpAndSettle();
      expect(
        find.byKey(const ValueKey('thread-message-group-reply-after-local-9')),
        findsOneWidget,
      );
    });

    testWidgets(
      'short followed thread stays top-anchored after viewport resize',
      (tester) async {
        tester.view.physicalSize = const Size(400, 800);
        tester.view.devicePixelRatio = 1;
        addTearDown(tester.view.reset);
        final rootEvent = _textMsg(
          id: 'thread-root',
          pubkey: 'alice',
          content: 'Root',
          createdAt: 1000,
        );
        final replies = [
          for (var i = 0; i < 3; i++)
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
              isMember: false,
              isArchived: false,
            ),
          ),
        );
        await tester.pumpAndSettle();

        final head = find.byKey(
          const ValueKey('thread-message-group-thread-root'),
        );
        final latest = find.byKey(
          const ValueKey('thread-message-group-reply-2'),
        );
        final initialHeadTop = tester.getTopLeft(head).dy;

        tester.view.viewInsets = const FakeViewPadding(bottom: 100);
        await tester.pumpAndSettle();

        expect(latest, findsOneWidget);
        expect(tester.getTopLeft(head).dy, closeTo(initialHeadTop, 0.5));
      },
    );

    for (final layout in <({String name, bool isMember, bool isArchived})>[
      (name: 'non-member', isMember: false, isArchived: false),
      (name: 'archived', isMember: true, isArchived: true),
      (name: 'writable member', isMember: true, isArchived: false),
    ]) {
      for (final tail in <({String name, bool isLong})>[
        (name: 'short', isLong: false),
        (name: 'long', isLong: true),
      ]) {
        testWidgets(
          '${layout.name} typing transitions preserve a followed ${tail.name} tail',
          (tester) async {
            tester.view.physicalSize = const Size(400, 800);
            tester.view.devicePixelRatio = 1;
            addTearDown(tester.view.reset);
            final rootEvent = _textMsg(
              id: 'thread-root',
              pubkey: 'alice',
              content: 'Root',
              createdAt: 1000,
            );
            final replyCount = tail.isLong ? 30 : 6;
            final replies = [
              for (var i = 0; i < replyCount; i++)
                _textMsg(
                  id: 'reply-$i',
                  pubkey: 'bob',
                  content: i == replyCount - 1 && tail.isLong
                      ? List.filled(8, 'Tall latest reply').join('\n')
                      : 'Reply $i',
                  createdAt: 1100 + i,
                  extraTags: const [
                    ['e', 'thread-root', '', 'reply'],
                  ],
                ),
            ];
            final typingNotifier = _FakeTypingNotifier(const []);
            await tester.pumpWidget(
              _buildTestable(
                messages: [rootEvent],
                typingNotifier: typingNotifier,
                threadReplies: {'thread-root': replies},
                disableAnimations: true,
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
                  isMember: layout.isMember,
                  isArchived: layout.isArchived,
                ),
              ),
            );
            await tester.pumpAndSettle();
            final latest = find.byKey(
              ValueKey('thread-message-group-reply-${replyCount - 1}'),
            );
            final head = find.byKey(
              const ValueKey('thread-message-group-thread-root'),
            );
            final initialHeadTop = tail.isLong
                ? null
                : tester.getTopLeft(head).dy;
            final list = find.byKey(const ValueKey('thread-message-list'));
            final initialHeight = tester.getSize(list).height;
            void expectTailWithinList() {
              final latestRect = tester.getRect(latest);
              final listRect = tester.getRect(list);
              expect(
                latestRect.top,
                greaterThanOrEqualTo(
                  frostedAppBarHeight(tester.element(latest)),
                ),
              );
              if (tail.isLong) {
                expect(latestRect.bottom, lessThanOrEqualTo(listRect.bottom));
              }
            }

            expectTailWithinList();
            typingNotifier.setEntries(const [
              TypingEntry(
                pubkey: 'bob',
                threadHeadId: 'thread-root',
                expiresAtMs: 9999999999999,
              ),
            ]);
            await tester.pumpAndSettle();
            expectTailWithinList();
            if (initialHeadTop != null) {
              expect(tester.getTopLeft(head).dy, closeTo(initialHeadTop, 0.5));
            }
            if (!layout.isMember || layout.isArchived) {
              expect(tester.getSize(list).height, lessThan(initialHeight));
            }
            typingNotifier.setEntries(const []);
            await tester.pumpAndSettle();
            expectTailWithinList();
            if (initialHeadTop != null) {
              expect(tester.getTopLeft(head).dy, closeTo(initialHeadTop, 0.5));
            }
            expect(tester.getSize(list).height, closeTo(initialHeight, 0.5));
          },
        );
      }

      testWidgets(
        '${layout.name} typing transitions preserve a detached anchor',
        (tester) async {
          tester.view.physicalSize = const Size(400, 800);
          tester.view.devicePixelRatio = 1;
          addTearDown(tester.view.reset);
          final rootEvent = _textMsg(
            id: 'thread-root',
            pubkey: 'alice',
            content: 'Root',
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
          final typingNotifier = _FakeTypingNotifier(const []);
          await tester.pumpWidget(
            _buildTestable(
              messages: [rootEvent],
              typingNotifier: typingNotifier,
              threadReplies: {'thread-root': replies},
              disableAnimations: true,
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
                isMember: layout.isMember,
                isArchived: layout.isArchived,
              ),
            ),
          );
          await tester.pumpAndSettle();
          final list = find.byKey(const ValueKey('thread-message-list'));
          for (var i = 0; i < 4; i++) {
            await tester.drag(list, const Offset(0, 100));
            await tester.pumpAndSettle();
          }
          final listRect = tester.getRect(list);
          final anchor =
              [
                for (var i = 0; i < replies.length; i++)
                  find.byKey(ValueKey('thread-message-group-reply-$i')),
              ].firstWhere(
                (candidate) =>
                    candidate.evaluate().length == 1 &&
                    listRect.overlaps(tester.getRect(candidate)),
              );
          final anchorTop = tester.getTopLeft(anchor).dy;
          typingNotifier.setEntries(const [
            TypingEntry(
              pubkey: 'bob',
              threadHeadId: 'thread-root',
              expiresAtMs: 9999999999999,
            ),
          ]);
          await tester.pumpAndSettle();
          expect(tester.getTopLeft(anchor).dy, closeTo(anchorTop, 0.5));
          typingNotifier.setEntries(const []);
          await tester.pumpAndSettle();
          expect(tester.getTopLeft(anchor).dy, closeTo(anchorTop, 0.5));
        },
      );
    }

    for (final layout in [
      (name: 'non-member', isMember: false, isArchived: false),
      (name: 'archived', isMember: true, isArchived: true),
    ]) {
      testWidgets('${layout.name} no-dock tail resumes full-viewport follow', (
        tester,
      ) async {
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
        final messagesNotifier = _FakeMessagesNotifier([rootEvent]);
        await tester.pumpWidget(
          _buildTestable(
            messages: [rootEvent],
            messagesNotifier: messagesNotifier,
            threadReplies: {'thread-root': replies},
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
              isMember: layout.isMember,
              isArchived: layout.isArchived,
            ),
          ),
        );
        await tester.pumpAndSettle();

        final lifecycle = tester.widget<KeyboardDismissOnDrag>(
          find.byType(KeyboardDismissOnDrag),
        );
        lifecycle.onUserScrollStart!();
        lifecycle.onUserScrollEnd!();
        await tester.pumpAndSettle();
        messagesNotifier.setMessages([
          rootEvent,
          _textMsg(
            id: 'reply-remote',
            pubkey: 'bob',
            content: 'Remote tail',
            createdAt: 9999,
            extraTags: const [
              ['e', 'thread-root', '', 'reply'],
            ],
          ),
        ]);
        await tester.pumpAndSettle();

        expect(
          find.byKey(const ValueKey('thread-message-group-reply-remote')),
          findsOneWidget,
        );
      });
    }

    testWidgets(
      'composer-covered tail remains detached after drag-end settlement',
      (tester) async {
        tester.view.physicalSize = const Size(400, 800);
        tester.view.devicePixelRatio = 1;
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
        final messagesNotifier = _FakeMessagesNotifier([rootEvent]);
        await tester.pumpWidget(
          _buildTestable(
            messages: [rootEvent],
            messagesNotifier: messagesNotifier,
            threadReplies: {'thread-root': replies},
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

        final list = find.byKey(const ValueKey('thread-message-list'));
        final latest = find.byKey(
          const ValueKey('thread-message-group-reply-29'),
        );
        final composer = find.byKey(const ValueKey('composer-surface'));
        // Clear the gesture arena's touch slop so this represents a deliberate
        // tail-detaching drag rather than a long-press hold with small motion.
        // The compact composer now rests lower, so use enough drag distance to
        // keep the final reply beneath its top edge for this covered-tail case.
        await tester.drag(list, const Offset(0, 56));
        await tester.pumpAndSettle();
        expect(
          tester.getBottomLeft(latest).dy,
          greaterThan(tester.getTopLeft(composer).dy),
        );
        expect(
          tester.getBottomLeft(latest).dy,
          lessThanOrEqualTo(tester.getBottomLeft(list).dy),
        );
        final visibleBeforeRemote = tester
            .widgetList<Widget>(
              find.byWidgetPredicate(
                (widget) =>
                    widget.key is ValueKey<String> &&
                    (widget.key! as ValueKey<String>).value.startsWith(
                      'thread-message-group-',
                    ),
              ),
            )
            .map((widget) => widget.key)
            .toSet();
        final anchorKey = visibleBeforeRemote.firstWhere(
          (key) => key != latest.evaluate().single.widget.key,
        );
        final anchor = find.byKey(anchorKey!);
        final detachedTop = tester.getTopLeft(anchor).dy;

        messagesNotifier.setMessages([
          rootEvent,
          _textMsg(
            id: 'reply-remote',
            pubkey: 'bob',
            content: 'Remote tail',
            createdAt: 9999,
            extraTags: const [
              ['e', 'thread-root', '', 'reply'],
            ],
          ),
        ]);
        await tester.pumpAndSettle();
        expect(
          tester
              .widgetList<Widget>(
                find.byWidgetPredicate(
                  (widget) => visibleBeforeRemote.contains(widget.key),
                ),
              )
              .map((widget) => widget.key),
          isNotEmpty,
        );
        expect(tester.getTopLeft(anchor).dy, closeTo(detachedTop, 0.5));

        await tester.tap(find.text('Reply in thread…').hitTestable());
        await tester.pumpAndSettle();
        expect(tester.getTopLeft(anchor).dy, closeTo(detachedTop, 0.5));

        tester.view.viewInsets = const FakeViewPadding(bottom: 300);
        await tester.pumpAndSettle();
        expect(tester.getTopLeft(anchor).dy, closeTo(detachedTop, 0.5));
      },
    );

    testWidgets('invalid writable dock geometry cannot resume tail follow', (
      tester,
    ) async {
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
      final messagesNotifier = _FakeMessagesNotifier([rootEvent]);
      await tester.pumpWidget(
        _buildTestable(
          messages: [rootEvent],
          messagesNotifier: messagesNotifier,
          threadReplies: {'thread-root': replies},
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

      tester
          .widget<ComposerDockSizeReporter>(
            find.byType(ComposerDockSizeReporter).last,
          )
          .onHeightChanged(0);
      await tester.pumpAndSettle();
      final lifecycle = tester.widget<KeyboardDismissOnDrag>(
        find.byType(KeyboardDismissOnDrag),
      );
      lifecycle.onUserScrollStart!();
      lifecycle.onUserScrollEnd!();
      await tester.pumpAndSettle();
      final anchor = find.byKey(
        const ValueKey('thread-message-group-reply-29'),
      );
      final anchorTop = tester.getTopLeft(anchor).dy;

      messagesNotifier.setMessages([
        rootEvent,
        _textMsg(
          id: 'reply-remote',
          pubkey: 'bob',
          content: 'Remote tail',
          createdAt: 9999,
          extraTags: const [
            ['e', 'thread-root', '', 'reply'],
          ],
        ),
      ]);
      await tester.pumpAndSettle();

      expect(tester.getTopLeft(anchor).dy, closeTo(anchorTop, 0.5));
    });

    testWidgets('new drag invalidates deferred drag-end resumption', (
      tester,
    ) async {
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
      final messagesNotifier = _FakeMessagesNotifier([rootEvent]);
      await tester.pumpWidget(
        _buildTestable(
          messages: [rootEvent],
          messagesNotifier: messagesNotifier,
          threadReplies: {'thread-root': replies},
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

      var lifecycle = tester.widget<KeyboardDismissOnDrag>(
        find.byType(KeyboardDismissOnDrag),
      );
      lifecycle.onUserScrollStart!();
      lifecycle.onUserScrollEnd!();
      await tester.pump();
      lifecycle = tester.widget<KeyboardDismissOnDrag>(
        find.byType(KeyboardDismissOnDrag),
      );
      lifecycle.onUserScrollStart!();
      await tester.pumpAndSettle();
      final anchor = find.byKey(
        const ValueKey('thread-message-group-reply-29'),
      );
      final anchorTop = tester.getTopLeft(anchor).dy;

      messagesNotifier.setMessages([
        rootEvent,
        _textMsg(
          id: 'reply-remote',
          pubkey: 'bob',
          content: 'Remote tail',
          createdAt: 9999,
          extraTags: const [
            ['e', 'thread-root', '', 'reply'],
          ],
        ),
      ]);
      await tester.pumpAndSettle();

      expect(tester.getTopLeft(anchor).dy, closeTo(anchorTop, 0.5));
      tester
          .widget<KeyboardDismissOnDrag>(find.byType(KeyboardDismissOnDrag))
          .onUserScrollEnd!();
    });

    testWidgets(
      'dragging away opts out, then returning to the tail resumes keyboard realignment',
      (tester) async {
        tester.view.physicalSize = const Size(400, 800);
        tester.view.devicePixelRatio = 1;
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
            ),
          ),
        );
        await tester.pumpAndSettle();

        final list = find.byKey(const ValueKey('thread-message-list'));
        for (var i = 0; i < 4; i++) {
          await tester.drag(list, const Offset(0, 100));
          await tester.pumpAndSettle();
        }
        expect(
          find.byKey(const ValueKey('thread-jump-to-latest')),
          findsOneWidget,
          reason: 'Browsing away from the tail should offer Latest.',
        );
        final visibleBeforeResize = tester
            .widgetList<Widget>(
              find.byWidgetPredicate(
                (widget) =>
                    widget.key is ValueKey<String> &&
                    (widget.key! as ValueKey<String>).value.startsWith(
                      'thread-message-group-',
                    ),
              ),
            )
            .map((widget) => widget.key)
            .toSet();

        tester.view.viewInsets = const FakeViewPadding(bottom: 300);
        await tester.pumpAndSettle();

        expect(
          find.byKey(const ValueKey('thread-message-group-reply-29')),
          findsNothing,
        );
        expect(
          tester
              .widgetList<Widget>(
                find.byWidgetPredicate(
                  (widget) => visibleBeforeResize.contains(widget.key),
                ),
              )
              .map((widget) => widget.key),
          isNotEmpty,
        );

        // Return the viewport to its original geometry while opt-out remains
        // active. This must not itself pull the list back to the tail.
        tester.view.viewInsets = FakeViewPadding.zero;
        await tester.pumpAndSettle();

        // Returning to the tail is a deliberate choice to resume following.
        // Only the scroll-end callback may clear the opt-out state, so this
        // reverse drag must complete before geometry changes re-align the tail.
        for (var i = 0; i < 12; i++) {
          await tester.drag(list, const Offset(0, -100));
          await tester.pumpAndSettle();
        }
        final latestReply = find.byKey(
          const ValueKey('thread-message-group-reply-29'),
        );
        expect(latestReply, findsOneWidget);

        final composerSurface = find.byKey(const ValueKey('composer-surface'));
        tester.view.viewInsets = const FakeViewPadding(bottom: 300);
        await tester.pumpAndSettle();

        expect(
          tester.getBottomLeft(latestReply).dy,
          lessThanOrEqualTo(tester.getTopLeft(composerSurface).dy),
        );
      },
    );

    testWidgets(
      'deep-link stays put through passive resize until composer focus follows tail',
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
        expect(
          find.byKey(const ValueKey('thread-jump-to-latest')),
          findsOneWidget,
        );

        tester.view.viewInsets = const FakeViewPadding(bottom: 300);
        await tester.pumpAndSettle();

        expect(target, findsOneWidget);

        await tester.tap(find.text('Reply in thread…').hitTestable());
        await tester.pumpAndSettle();

        expect(
          find.byKey(const ValueKey('thread-message-group-reply-29')),
          findsOneWidget,
        );
        expect(
          find.byKey(const ValueKey('thread-jump-to-latest')),
          findsNothing,
        );
      },
    );

    testWidgets('iOS composer focus and Latest fully reveal the final reply', (
      tester,
    ) async {
      final previousPlatform = debugDefaultTargetPlatformOverride;
      debugDefaultTargetPlatformOverride = TargetPlatform.iOS;
      addTearDown(() => debugDefaultTargetPlatformOverride = previousPlatform);
      tester.view.physicalSize = const Size(400, 800);
      tester.view.devicePixelRatio = 1;
      tester.view.viewPadding = const FakeViewPadding(bottom: 20);
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
          ),
        ),
      );
      await tester.pumpAndSettle();

      await tester.tap(find.text('Reply in thread…').hitTestable());
      await tester.pumpAndSettle();
      tester.view.viewInsets = const FakeViewPadding(bottom: 300);
      await tester.pumpAndSettle();

      final latestReply = find.byKey(
        const ValueKey('thread-message-group-reply-29'),
      );
      final composerSurface = find.byKey(const ValueKey('composer-surface'));
      final focusedReplyBottom = tester.getBottomLeft(latestReply).dy;
      final composerTop = tester.getTopLeft(composerSurface).dy;

      final list = find.byKey(const ValueKey('thread-message-list'));
      final listElement = tester.element(list);
      ScrollStartNotification(
        metrics: FixedScrollMetrics(
          minScrollExtent: 0,
          maxScrollExtent: 100,
          pixels: 0,
          viewportDimension: 100,
          axisDirection: AxisDirection.down,
          devicePixelRatio: 1,
        ),
        context: listElement,
        dragDetails: DragStartDetails(),
      ).dispatch(listElement);
      tester
          .widget<ScrollablePositionedList>(list)
          .itemScrollController!
          .jumpTo(index: 5);
      await tester.pumpAndSettle();
      final latestButton = find.byKey(const ValueKey('thread-jump-to-latest'));
      expect(
        latestReply,
        findsNothing,
        reason: 'Detaching from the tail should unmount the final reply.',
      );
      final latestButtonWasVisible = latestButton.evaluate().length == 1;
      final nativeView = tester.widget<UiKitView>(
        find.byKey(const ValueKey('thread-jump-to-latest-ios-glass')),
      );
      nativeView.onPlatformViewCreated!(42);
      await tester.pump();
      const nativeChannel = MethodChannel('buzz/jump_to_latest_glass/42');
      await tester.binding.defaultBinaryMessenger.handlePlatformMessage(
        nativeChannel.name,
        nativeChannel.codec.encodeMethodCall(const MethodCall('pressed')),
        (_) {},
      );
      await tester.pumpAndSettle();
      final latestReplyBottom = tester.getBottomLeft(latestReply).dy;
      debugDefaultTargetPlatformOverride = previousPlatform;

      expect(
        focusedReplyBottom,
        lessThanOrEqualTo(composerTop),
        reason: 'Focusing the composer must retain the final reply above it.',
      );
      expect(latestButtonWasVisible, isTrue);
      expect(
        latestReplyBottom,
        lessThanOrEqualTo(composerTop),
        reason: 'Latest must reveal the final reply above the iOS composer.',
      );
    });

    for (final platform in [TargetPlatform.android, TargetPlatform.iOS]) {
      testWidgets(
        'thread composer focus returns to the tail on ${platform.name}',
        (tester) async {
          final previousPlatform = debugDefaultTargetPlatformOverride;
          debugDefaultTargetPlatformOverride = platform;
          tester.view.physicalSize = const Size(400, 800);
          tester.view.devicePixelRatio = 1;
          tester.view.viewPadding = const FakeViewPadding(bottom: 20);
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
                initialMessageId: 'reply-5',
              ),
            ),
          );
          await tester.pumpAndSettle();
          expect(
            find.byKey(const ValueKey('thread-jump-to-latest')),
            findsOneWidget,
          );

          await tester.tap(find.text('Reply in thread…').hitTestable());
          await tester.pump();
          tester.view.viewInsets = const FakeViewPadding(bottom: 300);
          await tester.pump();
          if (platform == TargetPlatform.android) {
            await tester.pump(androidImeMetricsSettleDelay);
          }
          await tester.pumpAndSettle();

          final latestReply = find.byKey(
            const ValueKey('thread-message-group-reply-29'),
          );
          final composerSurface = find.byKey(
            const ValueKey('composer-surface'),
          );
          final focusNode = tester
              .widget<TextField>(find.byType(TextField))
              .focusNode!;
          final latestReplyBottom = tester.getBottomLeft(latestReply).dy;
          final composerTop = tester.getTopLeft(composerSurface).dy;
          final latestButtonIsVisible = find
              .byKey(const ValueKey('thread-jump-to-latest'))
              .evaluate()
              .isNotEmpty;
          debugDefaultTargetPlatformOverride = previousPlatform;

          expect(focusNode.hasFocus, isTrue);
          expect(latestReplyBottom, lessThanOrEqualTo(composerTop));
          expect(latestButtonIsVisible, isFalse);
        },
      );
    }

    testWidgets(
      'thread shows Latest after composer tail correction exhausts and focus leaves',
      (tester) async {
        tester.view.physicalSize = const Size(400, 800);
        tester.view.devicePixelRatio = 1;
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

        await tester.pumpWidget(
          _buildTestable(
            messages: [rootEvent],
            threadReplies: {'thread-root': replies},
            users: const {
              'alice': UserProfile(pubkey: 'alice', displayName: 'Alice'),
              'bob': UserProfile(pubkey: 'bob', displayName: 'Bob'),
            },
            home: ThreadDetailPage(
              threadHead: formatTimeline([rootEvent]).single,
              allMessages: formatTimeline([rootEvent, replies[5]]),
              channelId: _channelId,
              currentPubkey: 'self',
              isMember: true,
              isArchived: false,
              initialMessageId: 'reply-5',
              jumpThreadTailForTesting: () => true,
            ),
          ),
        );
        await tester.pumpAndSettle();

        const latestButton = ValueKey('thread-jump-to-latest');
        expect(find.byKey(latestButton), findsOneWidget);

        await tester.tap(find.text('Reply in thread…').hitTestable());
        await tester.pump();
        for (var frame = 0; frame < 10; frame++) {
          await tester.pump();
        }

        final focusNode = tester
            .widget<TextField>(find.byType(TextField))
            .focusNode!;
        expect(focusNode.hasFocus, isTrue);
        expect(
          find.byKey(const ValueKey('thread-message-group-reply-29')),
          findsNothing,
          reason: 'The lazy tail must remain unlaid after bounded correction.',
        );

        focusNode.unfocus();
        await tester.pumpAndSettle();

        expect(focusNode.hasFocus, isFalse);
        expect(find.byKey(latestButton), findsOneWidget);
      },
    );

    testWidgets('thread hides initial tail placement until it is settled', (
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
        for (var i = 0; i < 40; i++)
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

      Navigator.of(tester.element(find.byType(ChannelDetailPage))).push(
        MaterialPageRoute<void>(
          builder: (_) => ThreadDetailPage(
            threadHead: formatTimeline([rootEvent]).single,
            allMessages: formatTimeline([rootEvent, ...replies]),
            channelId: _channelId,
            currentPubkey: 'self',
            isMember: true,
            isArchived: false,
          ),
        ),
      );
      await tester.pump();
      await tester.pump();

      final gate = find.byKey(const ValueKey('thread-initial-viewport-gate'));
      expect(tester.widget<Opacity>(gate).opacity, 0);

      final list = find.byKey(const ValueKey('thread-message-list'));
      final scrollable = tester.state<ScrollableState>(
        find.descendant(of: list, matching: find.byType(Scrollable)).first,
      );
      expect(scrollable.position.isScrollingNotifier.value, isFalse);

      await tester.pumpAndSettle();

      expect(tester.widget<Opacity>(gate).opacity, 1);
      final latestReply = find.byKey(
        const ValueKey('thread-message-group-reply-39'),
      );
      expect(
        tester.getTopLeft(latestReply).dy,
        greaterThanOrEqualTo(frostedAppBarHeight(tester.element(latestReply))),
      );
      expect(
        tester.getTopLeft(latestReply).dy,
        lessThan(
          tester.getTopLeft(find.byKey(const ValueKey('composer-surface'))).dy,
        ),
      );
    });

    testWidgets('thread Latest reaches the tail after inbox hydration', (
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
        for (var i = 0; i < 40; i++)
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
      final authoritativeReplies = Completer<List<NostrEvent>>();

      await tester.pumpWidget(
        _buildTestable(
          messages: [rootEvent],
          pendingThreadReplies: {'thread-root': authoritativeReplies.future},
          localThreadReplies: {'thread-root': replies},
          users: const {
            'alice': UserProfile(pubkey: 'alice', displayName: 'Alice'),
            'bob': UserProfile(pubkey: 'bob', displayName: 'Bob'),
          },
          home: ThreadDetailPage(
            threadHead: formatTimeline([rootEvent]).single,
            allMessages: formatTimeline([rootEvent, replies[5]]),
            channelId: _channelId,
            currentPubkey: 'self',
            isMember: true,
            isArchived: false,
            initialMessageId: 'reply-5',
          ),
        ),
      );
      await tester.pumpAndSettle();

      expect(
        tester
            .widget<Opacity>(
              find.byKey(const ValueKey('thread-initial-viewport-gate')),
            )
            .opacity,
        1,
        reason:
            'Pending local replies must not make an in-flight relay query '
            'look hydrated and blank the route snapshot.',
      );
      expect(
        find.byKey(const ValueKey('thread-message-group-reply-5')),
        findsOneWidget,
      );

      authoritativeReplies.complete(replies);
      await tester.pumpAndSettle();

      expect(
        find.byKey(const ValueKey('thread-jump-to-latest')),
        findsOneWidget,
      );
      await tester.tap(find.byKey(const ValueKey('thread-jump-to-latest')));
      await tester.pumpAndSettle();
      expect(
        find.byKey(const ValueKey('thread-message-group-reply-39')),
        findsOneWidget,
      );
      final composerTop = tester
          .getTopLeft(find.byKey(const ValueKey('composer-surface')))
          .dy;
      expect(
        tester.getTopLeft(find.byKey(const ValueKey('thread-tail-anchor'))).dy,
        lessThanOrEqualTo(composerTop),
        reason: 'Latest must place the actual thread tail above the composer.',
      );
      expect(
        tester.getTopLeft(find.byKey(const ValueKey('thread-tail-anchor'))).dy,
        lessThanOrEqualTo(composerTop),
        reason: 'The hydrated Inbox thread must remain at its actual tail.',
      );
      expect(find.byKey(const ValueKey('thread-jump-to-latest')), findsNothing);
    });

    test('thread tail ignores oscillating item positions at exact extent', () {
      for (final tailItemIsVisible in [true, false, false, true, false]) {
        expect(
          threadTailIsAtEffectiveEnd(
            tailIsLaidOut: true,
            tailIsVisible: tailItemIsVisible,
            extentAfter: 0,
          ),
          isTrue,
        );
      }
      expect(
        threadTailIsAtEffectiveEnd(
          tailIsLaidOut: true,
          tailIsVisible: false,
          extentAfter: 1,
        ),
        isFalse,
      );
      expect(
        threadTailIsAtEffectiveEnd(
          tailIsLaidOut: false,
          tailIsVisible: false,
          extentAfter: 0,
        ),
        isFalse,
        reason: 'A not-yet-laid-out lazy tail cannot trust stale extent.',
      );
    });

    testWidgets('thread Latest settles across expanding lazy scroll extents', (
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
        for (var i = 0; i < 160; i++)
          _textMsg(
            id: 'reply-$i',
            pubkey: 'bob',
            content: [
              'Reply $i',
              ...List.filled(
                1 + (i ~/ 6),
                'Variable-height reply line for lazy layout.',
              ),
            ].join('\n'),
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
          home: ThreadDetailPage(
            threadHead: formatTimeline([rootEvent]).single,
            allMessages: formatTimeline([rootEvent, replies[5]]),
            channelId: _channelId,
            currentPubkey: 'self',
            isMember: true,
            isArchived: false,
            initialMessageId: 'reply-5',
          ),
        ),
      );
      await tester.pumpAndSettle();

      expect(
        find.byKey(const ValueKey('thread-message-group-reply-5')),
        findsOneWidget,
      );
      const latestButton = ValueKey('thread-jump-to-latest');
      expect(find.byKey(latestButton), findsOneWidget);
      final scrollable = tester.state<ScrollableState>(
        find
            .descendant(
              of: find.byKey(const ValueKey('thread-message-list')),
              matching: find.byType(Scrollable),
            )
            .first,
      );
      final initialMaxScrollExtent = scrollable.position.maxScrollExtent;

      await tester.tap(find.byKey(latestButton));
      await tester.pumpAndSettle();

      expect(
        scrollable.position.maxScrollExtent,
        greaterThan(initialMaxScrollExtent),
        reason:
            'The fixture must exercise a lazy extent that expands after '
            'Latest starts.',
      );
      expect(
        find.byKey(const ValueKey('thread-message-group-reply-159')),
        findsOneWidget,
      );
      expect(
        find.byKey(const ValueKey('thread-jump-to-latest-hidden')),
        findsOneWidget,
        reason:
            'Latest must keep correcting after the lazy extent expands '
            'beyond the first three layout frames.',
      );
    });

    testWidgets(
      'thread shows Latest after browsing history and returns to tail',
      (tester) async {
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
            ),
          ),
        );
        await tester.pumpAndSettle();

        final list = find.byKey(const ValueKey('thread-message-list'));
        expect(
          find.byKey(const ValueKey('thread-message-group-reply-29')),
          findsOneWidget,
        );
        expect(
          find.byKey(const ValueKey('thread-jump-to-latest')),
          findsNothing,
        );

        final landingScrollable = tester.state<ScrollableState>(
          find.descendant(of: list, matching: find.byType(Scrollable)).first,
        );
        landingScrollable.position.jumpTo(
          landingScrollable.position.maxScrollExtent - 24,
        );
        await tester.pump();
        expect(
          find.byKey(const ValueKey('thread-jump-to-latest')),
          findsNothing,
          reason:
              'A stale landing measurement must not expose Latest before the '
              'user explicitly browses history.',
        );

        await tester.drag(list, const Offset(0, 500));
        await tester.pumpAndSettle();

        expect(
          find.byKey(const ValueKey('thread-jump-to-latest')),
          findsOneWidget,
        );
        expect(
          tester.getSize(find.byKey(const ValueKey('thread-jump-to-latest'))),
          const Size.square(Grid.xl),
        );
        expect(
          tester.getSize(
            find.byKey(const ValueKey('thread-jump-to-latest-surface')),
          ),
          const Size.square(Grid.lg),
        );
        expect(
          find.descendant(
            of: find.byKey(const ValueKey('thread-jump-to-latest')),
            matching: find.byIcon(LucideIcons.arrowDown),
          ),
          findsOneWidget,
        );
        expect(find.text('Latest'), findsNothing);
        final threadLatestSwitcher = tester.widget<AnimatedSwitcher>(
          find.byKey(const ValueKey('thread-jump-to-latest-switcher')),
        );
        expect(
          threadLatestSwitcher.duration,
          const Duration(milliseconds: 180),
        );
        expect(
          threadLatestSwitcher.reverseDuration,
          const Duration(milliseconds: 160),
        );

        final threadScrollable = tester.state<ScrollableState>(
          find.descendant(of: list, matching: find.byType(Scrollable)).first,
        );
        final startPixels = threadScrollable.position.pixels;
        final targetPixels = threadScrollable.position.maxScrollExtent;
        await tester.tap(find.byKey(const ValueKey('thread-jump-to-latest')));
        await tester.pump();

        expect(
          find.byKey(const ValueKey('thread-jump-to-latest-hidden')),
          findsOneWidget,
          reason:
              'Latest should leave immediately once its navigation starts, '
              'rather than lingering over the thread at the tail.',
        );

        expect(
          find.descendant(of: list, matching: find.byType(Scrollable)),
          findsOneWidget,
          reason:
              'Latest must move the active thread scroll position directly; '
              'a second transitional list produces the visible bounce.',
        );

        expect(
          threadScrollable.position.isScrollingNotifier.value,
          isTrue,
          reason: 'Latest should use the same visible glide as the channel.',
        );
        await tester.pump(const Duration(milliseconds: 110));
        expect(threadScrollable.position.pixels, greaterThan(startPixels));
        expect(threadScrollable.position.pixels, lessThan(targetPixels));
        expect(threadScrollable.position.isScrollingNotifier.value, isTrue);
        await tester.pumpAndSettle();

        expect(
          find.byKey(const ValueKey('thread-message-group-reply-29')),
          findsOneWidget,
        );
        expect(
          find.byKey(const ValueKey('thread-jump-to-latest')),
          findsNothing,
        );
        final settledTailY = tester
            .getTopLeft(find.byKey(const ValueKey('thread-tail-anchor')))
            .dy;
        await tester.pump(const Duration(milliseconds: 250));
        expect(
          tester
              .getTopLeft(find.byKey(const ValueKey('thread-tail-anchor')))
              .dy,
          closeTo(settledTailY, 0.5),
          reason: 'Latest must not be followed by a corrective rebound.',
        );
      },
    );

    testWidgets(
      'a newly sent reply keeps Latest visible until the lazy tail settles',
      (tester) async {
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
          for (var i = 0; i < 160; i++)
            _textMsg(
              id: 'reply-$i',
              pubkey: 'bob',
              content: [
                'Reply $i',
                ...List.filled(
                  1 + (i ~/ 6),
                  'Variable-height reply line for lazy layout.',
                ),
              ].join('\n'),
              createdAt: 1100 + i,
              extraTags: const [
                ['e', 'thread-root', '', 'reply'],
              ],
            ),
        ];
        final messagesNotifier = _FakeMessagesNotifier([rootEvent]);

        await tester.pumpWidget(
          _buildTestable(
            messages: [rootEvent],
            messagesNotifier: messagesNotifier,
            threadReplies: {'thread-root': replies},
            users: const {
              'alice': UserProfile(pubkey: 'alice', displayName: 'Alice'),
              'bob': UserProfile(pubkey: 'bob', displayName: 'Bob'),
              'self': UserProfile(pubkey: 'self', displayName: 'Me'),
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

        final list = find.byKey(const ValueKey('thread-message-list'));
        final positionedList = tester.widget<ScrollablePositionedList>(list);
        final threadScrollable = tester.state<ScrollableState>(
          find.descendant(of: list, matching: find.byType(Scrollable)).first,
        );
        positionedList.itemScrollController!.jumpTo(index: 5);
        await tester.pumpAndSettle();
        await tester.drag(list, const Offset(0, 20));
        await tester.pumpAndSettle();
        expect(
          find.byKey(const ValueKey('thread-message-group-reply-159')),
          findsNothing,
        );
        final initialMaxScrollExtent =
            threadScrollable.position.maxScrollExtent;
        final localReply = _textMsg(
          id: 'reply-local',
          pubkey: 'self',
          content: 'My new reply\n${List.filled(30, 'Final line').join('\n')}',
          createdAt: 2000,
          extraTags: const [
            ['e', 'thread-root', '', 'reply'],
          ],
        );

        messagesNotifier.setMessages([rootEvent, localReply]);
        await tester.pump();
        await tester.pump();

        expect(
          find.byKey(const ValueKey('thread-jump-to-latest-hidden')),
          findsNothing,
          reason:
              'Automatic correction must not hide Latest before the lazy '
              'tail is actually visible.',
        );

        await tester.pumpAndSettle();

        expect(
          threadScrollable.position.maxScrollExtent,
          greaterThan(initialMaxScrollExtent),
          reason:
              'The fixture must expand the lazy extent after automatic '
              'correction starts.',
        );
        expect(
          find.byKey(const ValueKey('thread-message-group-reply-local')),
          findsOneWidget,
        );
        expect(
          threadScrollable.position.isScrollingNotifier.value,
          isFalse,
          reason: 'Reply-driven tail correction must be instant.',
        );
        expect(
          find.byKey(const ValueKey('thread-jump-to-latest')),
          findsNothing,
        );
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

class _FakeThreadLocalRepliesNotifier extends ThreadLocalRepliesNotifier {
  final List<NostrEvent> _replies;

  _FakeThreadLocalRepliesNotifier(super.args, this._replies);

  @override
  List<NostrEvent> build() => _replies;
}

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

class _TestAppLifecycleNotifier extends AppLifecycleNotifier {
  @override
  AppLifecycleState build() => AppLifecycleState.resumed;

  void setLifecycle(AppLifecycleState value) => state = value;
}

class _TrackingRelaySession extends RelaySessionNotifier {
  final visibleChannels = <String>[];

  @override
  SessionState build() =>
      const SessionState(status: SessionStatus.disconnected);

  @override
  void Function() registerVisibleChannel(String channelId) {
    final release = super.registerVisibleChannel(channelId);
    visibleChannels.add(channelId);
    var released = false;
    return () {
      if (released) return;
      released = true;
      visibleChannels.remove(channelId);
      release();
    };
  }
}

class _ReconnectingRelaySession extends RelaySessionNotifier {
  _ReconnectingRelaySession({
    this.huddleCreatePublishGate,
    this.huddleEndPublishGate,
  });

  final Future<void>? huddleCreatePublishGate;
  final Future<void>? huddleEndPublishGate;
  final huddleCreatePublishStarted = Completer<void>();
  final huddleEndPublishStarted = Completer<void>();
  final List<int> publishedKinds = [];

  @override
  SessionState build() =>
      const SessionState(status: SessionStatus.reconnecting);

  @override
  Future<List<NostrEvent>> fetchHistory(
    NostrFilter filter, {
    Duration timeout = const Duration(seconds: 8),
  }) async => [];

  @override
  Future<NostrEvent> publish(
    NostrEvent event, {
    Duration timeout = const Duration(seconds: 8),
  }) async {
    publishedKinds.add(event.kind);
    if (event.kind == 9007) {
      if (huddleCreatePublishGate case final gate?) {
        if (!huddleCreatePublishStarted.isCompleted) {
          huddleCreatePublishStarted.complete();
        }
        await gate;
      }
    }
    if (event.kind == EventKind.huddleEnded) {
      if (huddleEndPublishGate case final gate?) {
        if (!huddleEndPublishStarted.isCompleted) {
          huddleEndPublishStarted.complete();
        }
        await gate;
      }
    }
    return event;
  }

  void connect() {
    state = const SessionState(status: SessionStatus.connected);
  }
}

class _IdentityUpdateRelaySession extends RelaySessionNotifier {
  _IdentityUpdateRelaySession({this.profileRefresh});

  final Future<List<NostrEvent>>? profileRefresh;
  NostrFilter? identityFilter;
  NostrFilter? directIdentityFilter;
  List<NostrEvent> directIdentityProfiles = const [];
  void Function(NostrEvent)? _identityListener;
  void Function(String message)? _identityClosedListener;
  void Function(RelaySubscriptionStatus status)? _identityStatusListener;
  void Function(NostrEvent)? _membershipListener;
  void Function(RelaySubscriptionStatus status)? _membershipStatusListener;
  NostrEvent? membershipSnapshot;

  @override
  SessionState build() => const SessionState(status: SessionStatus.connected);

  @override
  Future<List<NostrEvent>> fetchHistory(
    NostrFilter filter, {
    Duration timeout = const Duration(seconds: 8),
  }) async {
    if (filter.kinds.length == 1 && filter.kinds.single == 0) {
      return profileRefresh ?? const [];
    }
    if (filter.kinds.contains(10100) && filter.kinds.length == 1) {
      directIdentityFilter = filter;
      return directIdentityProfiles;
    }
    return membershipSnapshot == null ? const [] : [membershipSnapshot!];
  }

  @override
  Future<void Function()> subscribeWithStatus(
    NostrFilter filter,
    void Function(NostrEvent) onEvent, {
    void Function(String message)? onClosed,
    required void Function(RelaySubscriptionStatus status) onStatusChanged,
  }) async {
    if (filter.kinds.contains(10100)) {
      identityFilter = filter;
      _identityListener = onEvent;
      _identityClosedListener = onClosed;
      _identityStatusListener = onStatusChanged;
      onStatusChanged(RelaySubscriptionStatus.ready);
      return () {
        if (identical(_identityListener, onEvent)) {
          _identityListener = null;
          _identityClosedListener = null;
          _identityStatusListener = null;
        }
      };
    }
    _membershipListener = onEvent;
    _membershipStatusListener = onStatusChanged;
    onStatusChanged(RelaySubscriptionStatus.ready);
    return () {
      if (identical(_membershipListener, onEvent)) {
        _membershipListener = null;
        _membershipStatusListener = null;
      }
    };
  }

  @override
  Future<void Function()> subscribe(
    NostrFilter filter,
    void Function(NostrEvent) onEvent, {
    void Function(String message)? onClosed,
  }) async {
    if (filter.kinds.contains(10100)) {
      identityFilter = filter;
      _identityListener = onEvent;
      _identityClosedListener = onClosed;
    }
    return () {
      if (identical(_identityListener, onEvent)) {
        _identityListener = null;
        _identityClosedListener = null;
      }
    };
  }

  void emitProfile(NostrEvent event) {
    _identityListener?.call(event);
  }

  void emitAgentProfile({required String pubkey}) {
    _identityListener?.call(
      NostrEvent(
        id: 'agent-profile-$pubkey',
        pubkey: pubkey,
        createdAt: DateTime.now().millisecondsSinceEpoch ~/ 1000,
        kind: 10100,
        tags: const [],
        content: '{"name":"Agent"}',
        sig: 'sig',
      ),
    );
  }

  void closeIdentitySubscription() {
    _identityClosedListener?.call('unsupported filter');
  }

  void retryIdentitySubscription() {
    _identityStatusListener?.call(RelaySubscriptionStatus.retrying);
  }

  void readyIdentitySubscription() {
    _identityStatusListener?.call(RelaySubscriptionStatus.ready);
  }

  void beginMembershipReplay() {
    _membershipStatusListener?.call(RelaySubscriptionStatus.retrying);
  }

  void emitReplayedMembership(NostrEvent event) {
    membershipSnapshot = event;
    _membershipListener?.call(event);
    _membershipStatusListener?.call(RelaySubscriptionStatus.retrying);
  }

  void finishMembershipReplay() {
    _membershipStatusListener?.call(RelaySubscriptionStatus.ready);
  }

  void disconnect() {
    state = const SessionState(status: SessionStatus.disconnected);
  }

  void connect() {
    state = const SessionState(status: SessionStatus.connected);
  }
}

class _ProfileSubscriptionRelaySession extends RelaySessionNotifier {
  int profileSubscriptions = 0;

  @override
  SessionState build() => const SessionState(status: SessionStatus.connected);

  @override
  Future<List<NostrEvent>> fetchHistory(
    NostrFilter filter, {
    Duration timeout = const Duration(seconds: 8),
  }) async => const [];

  @override
  Future<NostrEvent> publish(
    NostrEvent event, {
    Duration timeout = const Duration(seconds: 8),
  }) async => event;

  @override
  Future<void Function()> subscribe(
    NostrFilter filter,
    void Function(NostrEvent) onEvent, {
    void Function(String message)? onClosed,
  }) async {
    if (filter.kinds.contains(0)) profileSubscriptions++;
    return () {};
  }
}

class _HuddleReactionRelaySession extends RelaySessionNotifier {
  NostrFilter? reactionFilter;
  void Function(NostrEvent)? _reactionListener;
  var _nextEventId = 0;

  @override
  SessionState build() => const SessionState(status: SessionStatus.connected);

  @override
  Future<List<NostrEvent>> fetchHistory(
    NostrFilter filter, {
    Duration timeout = const Duration(seconds: 8),
  }) async => const [];

  @override
  Future<NostrEvent> publish(
    NostrEvent event, {
    Duration timeout = const Duration(seconds: 8),
  }) async => event;

  @override
  Future<void Function()> subscribe(
    NostrFilter filter,
    void Function(NostrEvent) onEvent, {
    void Function(String message)? onClosed,
  }) async {
    if (filter.kinds.contains(EventKind.huddleReaction)) {
      reactionFilter = filter;
      _reactionListener = onEvent;
      return () {
        if (identical(_reactionListener, onEvent)) {
          _reactionListener = null;
        }
      };
    }
    return () {};
  }

  void emitReaction({required String pubkey, required String emoji}) {
    _reactionListener?.call(
      NostrEvent(
        id: 'huddle-reaction-${_nextEventId++}',
        pubkey: pubkey,
        createdAt: DateTime.now().millisecondsSinceEpoch ~/ 1000,
        kind: EventKind.huddleReaction,
        tags: [
          ['h', _huddleChannelId],
          ['reaction', emoji],
          ['sender_name', 'Remote'],
        ],
        content: emoji,
        sig: 'sig',
      ),
    );
  }
}

class _FakeTypingNotifier extends ChannelTypingNotifier {
  final List<TypingEntry> _entries;
  _FakeTypingNotifier(this._entries, {String channelId = _channelId})
    : super(channelId);

  @override
  List<TypingEntry> build() => _entries;

  void setEntries(List<TypingEntry> entries) => state = entries;
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

class _FakeChannelStarsNotifier extends ChannelStarsNotifier {
  @override
  ChannelStarsState build() => const ChannelStarsState(isReady: true);

  @override
  void starChannel(String channelId) => _setStarred(channelId, true);

  @override
  void unstarChannel(String channelId) => _setStarred(channelId, false);

  void _setStarred(String channelId, bool starred) {
    state = ChannelStarsState(
      isReady: true,
      store: ChannelStarStore(
        channels: {
          ...state.store.channels,
          channelId: ChannelStarEntry(starred: starred, updatedAt: 1),
        },
      ),
      version: state.version + 1,
    );
  }
}

class _FakeChannelMutesNotifier extends ChannelMutesNotifier {
  @override
  ChannelMutesState build() => const ChannelMutesState(isReady: true);

  @override
  void muteChannel(String channelId) => _setMuted(channelId, true);

  @override
  void unmuteChannel(String channelId) => _setMuted(channelId, false);

  void _setMuted(String channelId, bool muted) {
    state = ChannelMutesState(
      isReady: true,
      store: ChannelMuteStore(
        channels: {
          ...state.store.channels,
          channelId: ChannelMuteEntry(muted: muted, updatedAt: 1),
        },
      ),
      version: state.version + 1,
    );
  }
}

NostrEvent _profileEvent({
  required String id,
  required String pubkey,
  required int createdAt,
  required String name,
  List<List<String>> tags = const [],
}) => NostrEvent(
  id: id,
  pubkey: pubkey,
  createdAt: createdAt,
  kind: 0,
  tags: tags,
  content: jsonEncode({'name': name}),
  sig: 'sig',
);

List<String> _authTag(nostr.Keys owner, String agentPubkey) {
  final digest = SHA256Digest().process(
    Uint8List.fromList(
      utf8.encode('nostr:agent-auth:${agentPubkey.toLowerCase()}:'),
    ),
  );
  final message = digest
      .map((byte) => byte.toRadixString(16).padLeft(2, '0'))
      .join();
  return [
    'auth',
    owner.public,
    '',
    nostr.Schnorr.sign(secretKey: owner.secret, message: message),
  ];
}

class _FakeUserCacheNotifier extends UserCacheNotifier {
  final Map<String, UserProfile> _users;
  final Future<bool> Function(List<String>)? _preload;
  _FakeUserCacheNotifier(
    this._users, {
    Future<bool> Function(List<String>)? preload,
  }) : _preload = preload;

  @override
  Map<String, UserProfile> build() => _users;

  @override
  UserProfile? get(String pubkey) => _users[pubkey.toLowerCase()];

  @override
  Future<bool> preload(List<String> pubkeys) =>
      _preload?.call(pubkeys) ?? Future.value(true);

  @override
  Future<bool> refresh(List<String> pubkeys) => preload(pubkeys);

  void replace(UserProfile profile) {
    state = {...state, profile.pubkey.toLowerCase(): profile};
  }
}

class _MutableHuddleMembersNotifier extends Notifier<List<ChannelMember>> {
  _MutableHuddleMembersNotifier(this._initialMembers);

  final List<ChannelMember> _initialMembers;

  @override
  List<ChannelMember> build() => _initialMembers;

  void replace(List<ChannelMember> members) => state = members;
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
  final Future<void> Function(String channelId)? onLeaveChannel;
  final Future<void> Function(String channelId)? onArchiveChannel;
  final Future<void> Function(String channelId, List<String> pubkeys)?
  onAddMembers;
  final Future<void> Function(
    String channelId,
    String? name,
    String? description,
  )?
  onUpdateChannel;

  _FakeChannelActions(
    Ref ref, {
    this.onJoinChannel,
    this.onLeaveChannel,
    this.onArchiveChannel,
    this.onAddMembers,
    this.onUpdateChannel,
  }) : super(
         ref: ref,
         session: ref.read(relaySessionProvider.notifier),
         signedEventRelay: SignedEventRelay(
           session: ref.read(relaySessionProvider.notifier),
           nsec: ref.read(relayConfigProvider).nsec,
         ),
         currentPubkey: 'self',
       );

  @override
  Future<void> joinChannel(String channelId) async {
    await onJoinChannel?.call(channelId);
  }

  @override
  Future<void> addMembers({
    required String channelId,
    required List<String> pubkeys,
    String role = 'member',
  }) async {
    await onAddMembers?.call(channelId, pubkeys);
  }

  @override
  Future<void> leaveChannel(String channelId) async {
    await onLeaveChannel?.call(channelId);
  }

  @override
  Future<void> archiveChannel(String channelId) async {
    await onArchiveChannel?.call(channelId);
  }

  @override
  Future<void> updateChannel({
    required String channelId,
    String? name,
    String? description,
  }) async {
    await onUpdateChannel?.call(channelId, name, description);
  }
}

class _RecordingNavigatorObserver extends NavigatorObserver {
  final List<Route<dynamic>> pushedRoutes = [];

  @override
  void didPush(Route<dynamic> route, Route<dynamic>? previousRoute) {
    super.didPush(route, previousRoute);
    pushedRoutes.add(route);
  }
}

class _HuddleRelayConfigNotifier extends RelayConfigNotifier {
  final String _nsec = nostr.Keys.generate().nsec;

  @override
  RelayConfig build() =>
      RelayConfig(baseUrl: 'https://relay.example', nsec: _nsec);
}

final class _HuddleTestMedia implements HuddleMedia {
  _HuddleTestMedia({
    this.stopGate,
    this.permission = HuddleMicrophonePermission.granted,
  });

  final Future<void>? stopGate;
  final HuddleMicrophonePermission permission;
  final stopStarted = Completer<void>();
  final _states = StreamController<HuddleMediaState>.broadcast(sync: true);
  final _localFrames = StreamController<HuddleLocalAudioFrame>.broadcast(
    sync: true,
  );
  HuddleMediaState _state = const HuddleMediaState(
    phase: HuddleMediaPhase.idle,
  );

  @override
  HuddleMediaState get state => _state;

  @override
  Stream<HuddleMediaState> get states => _states.stream;

  @override
  Stream<HuddleLocalAudioFrame> get localAudioFrames => _localFrames.stream;

  @override
  Future<HuddleMediaCapabilities> discoverCapabilities() async {
    const capabilities = HuddleMediaCapabilities(
      platform: 'test',
      supportsAudioSession: true,
      supportsMicrophonePermission: true,
      supportsCapture: true,
      supportsPlayback: true,
      supportsOpusEncoding: true,
      supportsOpusDecoding: true,
    );
    _state = const HuddleMediaState(
      phase: HuddleMediaPhase.idle,
      capabilities: capabilities,
    );
    return capabilities;
  }

  @override
  Future<HuddleMicrophonePermission> requestMicrophonePermission() async =>
      permission;

  var openSettingsCalls = 0;

  @override
  Future<bool> openSystemSettings() async {
    openSettingsCalls += 1;
    return true;
  }

  @override
  Future<void> prepare() async {
    _emit(
      HuddleMediaState(
        phase: HuddleMediaPhase.prepared,
        capabilities: _state.capabilities,
      ),
    );
  }

  @override
  Future<void> start() async {
    _emit(
      HuddleMediaState(
        phase: HuddleMediaPhase.active,
        capabilities: _state.capabilities,
      ),
    );
  }

  @override
  Future<void> setMuted(bool muted) async {
    _emit(
      HuddleMediaState(
        phase: HuddleMediaPhase.active,
        capabilities: _state.capabilities,
        isMuted: muted,
        isSpeakerEnabled: _state.isSpeakerEnabled,
      ),
    );
  }

  @override
  Future<void> setSpeakerEnabled(bool enabled) async {
    _emit(
      HuddleMediaState(
        phase: HuddleMediaPhase.active,
        capabilities: _state.capabilities,
        isMuted: _state.isMuted,
        isSpeakerEnabled: enabled,
      ),
    );
  }

  void emitFailure() {
    scheduleMicrotask(() {
      _emit(
        HuddleMediaState(
          phase: HuddleMediaPhase.failed,
          capabilities: _state.capabilities,
          error: const HuddleMediaError(
            code: HuddleMediaErrorCode.platformFailure,
            message: 'Native audio failed.',
          ),
        ),
      );
    });
  }

  @override
  Future<void> playRemoteFrame(HuddleRemoteAudioFrame frame) async {}

  @override
  Future<void> removeRemotePeer(int peerIndex) async {}

  @override
  Future<void> stop() async {
    if (!stopStarted.isCompleted) stopStarted.complete();
    if (stopGate case final gate?) await gate;
    scheduleMicrotask(() {
      _emit(
        HuddleMediaState(
          phase: HuddleMediaPhase.stopped,
          capabilities: _state.capabilities,
        ),
      );
    });
  }

  @override
  Future<void> dispose() => stop();

  void _emit(HuddleMediaState state) {
    _state = state;
    _states.add(state);
  }
}

final class _HuddleTestTransport implements HuddleTransportClient {
  _HuddleTestTransport({
    this.connectError,
    this.connectGate,
    Map<int, HuddlePeer> peers = const {
      1: HuddlePeer(pubkey: 'desktop', peerIndex: 1, epoch: 0),
      2: HuddlePeer(pubkey: 'self', peerIndex: 2, epoch: 0),
    },
  }) : _peers = Map<int, HuddlePeer>.from(peers);

  final HuddleTransportError? connectError;
  final Future<void>? connectGate;
  final Map<int, HuddlePeer> _peers;
  final _states = StreamController<HuddleTransportState>.broadcast(sync: true);
  final _remoteFrames = StreamController<HuddleRemoteAudioFrame>.broadcast(
    sync: true,
  );
  final _peerEvents = StreamController<HuddlePeerEvent>.broadcast(sync: true);
  final _issues = StreamController<HuddleTransportError>.broadcast(sync: true);
  HuddleTransportState _state = HuddleTransportState.idle();

  void emitPeerJoin(HuddlePeer peer) {
    _peers[peer.peerIndex] = peer;
    _state = HuddleTransportState(
      phase: HuddleTransportPhase.connected,
      localPeerIndex: _state.localPeerIndex,
      peers: _peers,
    );
    _states.add(_state);
    _peerEvents.add(
      HuddlePeerEvent(type: HuddlePeerEventType.joined, peer: peer),
    );
  }

  void emitPeerLeave(int peerIndex) {
    final peer = _peers.remove(peerIndex);
    if (peer == null) return;
    _state = HuddleTransportState(
      phase: HuddleTransportPhase.connected,
      localPeerIndex: _state.localPeerIndex,
      peers: _peers,
    );
    _states.add(_state);
    _peerEvents.add(
      HuddlePeerEvent(type: HuddlePeerEventType.left, peer: peer),
    );
  }

  void emitRemoteAudio({
    int peerIndex = 1,
    int levelDbov = -30,
    int sequence = 1,
  }) {
    _remoteFrames.add(
      HuddleRemoteAudioFrame(
        peerIndex: peerIndex,
        epoch: 0,
        header: HuddleAudioHeader(
          sequence: sequence,
          timestamp48k: 960,
          levelDbov: levelDbov,
          flags: 0,
        ),
        opusPayload: Uint8List.fromList([1, 2, 3]),
      ),
    );
  }

  @override
  HuddleTransportState get state => _state;

  @override
  Stream<HuddleTransportState> get states => _states.stream;

  @override
  Stream<HuddleRemoteAudioFrame> get remoteAudioFrames => _remoteFrames.stream;

  @override
  Stream<HuddlePeerEvent> get peerEvents => _peerEvents.stream;

  @override
  Stream<HuddleTransportError> get issues => _issues.stream;

  @override
  Future<void> connect() async {
    if (connectGate case final gate?) await gate;
    if (connectError case final error?) throw error;
    _state = HuddleTransportState(
      phase: HuddleTransportPhase.connected,
      localPeerIndex: 2,
      peers: _peers,
    );
    _states.add(_state);
  }

  @override
  void sendOpusFrame({
    required HuddleAudioHeader header,
    required Uint8List opusPayload,
  }) {}

  @override
  Future<void> disconnect() async {
    _state = HuddleTransportState(phase: HuddleTransportPhase.disconnected);
    _states.add(_state);
  }

  @override
  Future<void> dispose() => disconnect();
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

final _transparentPng = base64Decode(
  'iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mNk+A8AAQUBAScY42YAAAAASUVORK5CYII=',
);

class _TestNavigatorObserver extends NavigatorObserver {
  int pushCount = 0;

  @override
  void didPush(Route<dynamic> route, Route<dynamic>? previousRoute) {
    pushCount += 1;
    super.didPush(route, previousRoute);
  }
}
