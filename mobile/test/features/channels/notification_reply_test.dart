import 'package:buzz/features/activity/compose_drafts_provider.dart';
import 'package:buzz/features/channels/channel.dart';
import 'package:buzz/features/channels/channels_provider.dart';
import 'package:buzz/features/channels/notification_reply.dart';
import 'package:buzz/features/channels/send_message_provider.dart';
import 'package:buzz/shared/auth/auth.dart';
import 'package:buzz/shared/push/push_bridge.dart';
import 'package:buzz/shared/read_state/read_state_format.dart';
import 'package:buzz/shared/read_state/read_state_provider.dart';
import 'package:buzz/shared/relay/relay.dart';
import 'package:flutter/foundation.dart';
import 'package:flutter/services.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';

const _push = MethodChannel('buzz/push');
final _root = 'a' * 64;
final _message = 'b' * 64;
final _sender = 'c' * 64;

final _community = Community(
  id: 'community',
  name: 'Example',
  relayUrl: 'https://relay.example.org',
  addedAt: DateTime.utc(2026),
);

Channel _channel(String id, {bool solo = false, bool member = true}) => Channel(
  id: id,
  name: id,
  description: '',
  channelType: 'stream',
  visibility: solo ? 'private' : 'open',
  createdBy: 'owner',
  createdAt: DateTime.utc(2026),
  memberCount: 2,
  isMember: member,
  labels: solo ? const ['buzz.solo'] : const [],
);

Map<String, String> _reply(
  String id, {
  String communityId = 'community',
  String channelId = 'general',
  String? threadRootId,
  String? replyRootId,
  int createdAt = 1000,
}) => {
  'id': id,
  'text': 'reply $id',
  'eventId': _message,
  'communityId': communityId,
  'channelId': channelId,
  'threadRootId': ?threadRootId,
  'senderPubkey': _sender,
  'replyRootId': ?replyRootId,
  'messageCreatedAt': '$createdAt',
};

void main() {
  TestWidgetsFlutterBinding.ensureInitialized();

  test(
    'replies typed into notifications are sent, read and reported',
    () async {
      debugDefaultTargetPlatformOverride = TargetPlatform.iOS;
      final messenger =
          TestDefaultBinaryMessengerBinding.instance.defaultBinaryMessenger;
      final waiting = <Map<String, String>>[
        // A top-level channel message: a new subthread under it.
        _reply('1'),
        // A message inside a thread: the subthread keeps the thread's root.
        _reply('2', threadRootId: _root, replyRootId: _root, createdAt: 2000),
        // A Solo chat takes no thread replies.
        _reply('3', channelId: 'solo', createdAt: 3000),
        // Another community's notification can't be answered without
        // switching; its text is kept as a draft.
        _reply('4', communityId: 'elsewhere'),
        // A send that fails keeps the text where its notification opens.
        _reply(
          '5',
          channelId: 'broken',
          threadRootId: _root,
          replyRootId: _root,
        ),
        // A channel the user isn't in isn't replied to, and its text goes
        // after the draft already there.
        _reply('6', channelId: 'outside'),
      ];
      installBuzzPushMethodHandler();
      final outcomes = <String, bool>{};
      final selected = <Object?>[];
      messenger.setMockMethodCallHandler(_push, (call) async {
        switch (call.method) {
          case 'takePendingNotificationReplies':
            final taken = [...waiting];
            waiting.clear();
            return taken;
          case 'completeNotificationReply':
            final args = call.arguments as Map;
            outcomes[args['id'] as String] = args['sent'] as bool;
          case 'setSelectedCommunity':
            selected.add(call.arguments);
        }
        return null;
      });
      addTearDown(() {
        messenger.setMockMethodCallHandler(_push, null);
        debugDefaultTargetPlatformOverride = null;
      });

      final sender = _RecordingSender();
      final drafts = _Drafts();
      final session = _Session();
      final container = ProviderContainer(
        overrides: [
          authProvider.overrideWith(_Auth.new),
          activeCommunityProvider.overrideWith((ref) async => _community),
          channelsProvider.overrideWith(
            () => _Channels([
              _channel('general'),
              _channel('solo', solo: true),
              _channel('broken'),
              _channel('outside', member: false),
            ]),
          ),
          sendMessageProvider.overrideWithValue(sender),
          readStateProvider.overrideWith(_ReadState.new),
          composeDraftsProvider.overrideWith(() => drafts),
          relaySessionProvider.overrideWith(() => session),
        ],
      );
      addTearDown(container.dispose);

      container.listen(notificationReplySenderProvider, (_, _) {});
      await pumpEventQueue();

      expect(selected, ['community']);
      expect(outcomes, {
        '1': true,
        '2': true,
        '3': true,
        '4': false,
        '5': false,
        '6': false,
      });
      expect(sender.calls, [
        ('general', 'reply 1', _message, _message, _sender),
        ('general', 'reply 2', _message, _root, _sender),
        ('solo', 'reply 3', null, null, _sender),
        ('broken', 'reply 5', _message, _root, _sender),
      ]);
      // Read only up to each notified message, so newer ones stay unread.
      expect(container.read(readStateProvider).contexts, {
        'general': 1000,
        threadContextKey(_root): 2000,
        'solo': 3000,
      });
      expect(drafts.textFor(composeDraftKey('general')), 'reply 4');
      expect(
        drafts.textFor(composeDraftKey('broken', threadHeadId: _root)),
        'reply 5',
      );
      expect(
        drafts.textFor(composeDraftKey('outside')),
        'earlier draft\n\nreply 6',
      );

      Future<void> signalReply(Map<String, String> reply) async {
        waiting.add(reply);
        await messenger.handlePlatformMessage(
          _push.name,
          _push.codec.encodeMethodCall(
            const MethodCall('notificationReplyAvailable'),
          ),
          (_) {},
        );
        await pumpEventQueue();
      }

      // A reply arriving while the app runs is sent when iOS signals it.
      await signalReply(_reply('7', channelId: 'solo'));
      expect(outcomes['7'], isTrue);

      // When the relay can't be reached in time, nothing is sent and the
      // text is kept.
      session.reachable = false;
      await signalReply(_reply('8', channelId: 'solo'));
      expect(outcomes['8'], isFalse);
      expect(sender.calls.last.$2, 'reply 7');
      expect(drafts.textFor(composeDraftKey('solo')), 'reply 8');
    },
  );
}

class _Auth extends AuthNotifier {
  @override
  Future<AuthState> build() async =>
      AuthState(status: AuthStatus.authenticated, community: _community);
}

class _Channels extends ChannelsNotifier {
  _Channels(this._channels);
  final List<Channel> _channels;
  @override
  Future<List<Channel>> build() async => _channels;
}

class _ReadState extends ReadStateNotifier {
  @override
  ReadStateState build() => const ReadStateState(
    isReady: true,
    pubkey: 'me',
    contexts: {},
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

class _Drafts extends ComposeDraftsNotifier {
  @override
  List<ComposeDraft> build() => [
    ComposeDraft(
      key: composeDraftKey('outside'),
      channelId: 'outside',
      threadHeadId: null,
      text: 'earlier draft',
      updatedAt: 1,
    ),
  ];
  @override
  void save({
    required String key,
    required String channelId,
    String? threadHeadId,
    required String text,
    Map<String, String> mentionKeys = const {},
  }) => state = [
    ComposeDraft(
      key: key,
      channelId: channelId,
      threadHeadId: threadHeadId,
      text: text,
      updatedAt: 2,
    ),
    ...state.where((draft) => draft.key != key),
  ];
}

class _Session extends RelaySessionNotifier {
  var reachable = true;
  @override
  SessionState build() =>
      const SessionState(status: SessionStatus.disconnected);
  @override
  Future<bool> connectForBackgroundWork({required Duration timeout}) async =>
      reachable;
}

class _RecordingSender implements SendMessage {
  final calls = <(String, String, String?, String?, String?)>[];
  @override
  Future<void> call({
    required String channelId,
    required String content,
    String? parentEventId,
    String? rootEventId,
    List<String>? mentionPubkeys,
    Channel? channel,
    List<List<String>> mediaTags = const [],
  }) async {
    calls.add((
      channelId,
      content,
      parentEventId,
      rootEventId,
      mentionPubkeys?.join(','),
    ));
    if (channelId == 'broken') throw StateError('relay rejected');
  }
}
