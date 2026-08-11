import 'dart:async';

import 'package:flutter_test/flutter_test.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';
import 'package:nostr/nostr.dart' as nostr;
import 'package:buzz/features/channels/channel_management_provider.dart';
import 'package:buzz/features/channels/send_message_provider.dart';
import 'package:buzz/shared/relay/relay.dart';

void main() {
  test(
    'adds the signed message locally before relay acknowledgement',
    () async {
      final session = _PendingPublishRelaySession();
      final localMessages = <NostrEvent>[];
      final removedIds = <String>[];
      final completedIds = <String>[];
      final send = SendMessage(
        signedEventRelay: SignedEventRelay(
          session: session,
          nsec: nostr.Keys.generate().nsec,
        ),
        fetchMembers: (_) async => const [],
        readUserCache: () => const {},
        addLocalMessage: (_, event) => localMessages.add(event),
        completeLocalMessage: (_, eventId) => completedIds.add(eventId),
        removeLocalMessage: (_, eventId) => removedIds.add(eventId),
      );

      final result = send(channelId: _channelId, content: 'hello');
      await session.published;

      expect(localMessages, hasLength(1));
      expect(localMessages.single.id, session.event.id);
      expect(localMessages.single.content, 'hello');
      expect(localMessages.single.channelId, _channelId);
      expect(removedIds, isEmpty);

      session.accept();
      await result;
      expect(completedIds, [localMessages.single.id]);
      expect(removedIds, isEmpty);
    },
  );

  test('rolls back the signed local message when publish fails', () async {
    final session = _PendingPublishRelaySession();
    final localMessages = <NostrEvent>[];
    final completedIds = <String>[];
    final removedIds = <String>[];
    final send = SendMessage(
      signedEventRelay: SignedEventRelay(
        session: session,
        nsec: nostr.Keys.generate().nsec,
      ),
      fetchMembers: (_) async => const [],
      readUserCache: () => const {},
      addLocalMessage: (_, event) => localMessages.add(event),
      completeLocalMessage: (_, eventId) => completedIds.add(eventId),
      removeLocalMessage: (_, eventId) => removedIds.add(eventId),
    );

    final result = send(channelId: _channelId, content: 'hello');
    await session.published;
    session.reject();

    await expectLater(result, throwsException);
    expect(completedIds, isEmpty);
    expect(removedIds, [localMessages.single.id]);
  });

  test('plain DM messages p-tag every recipient except the sender', () async {
    final session = _PendingPublishRelaySession();
    final signer = nostr.Keys.generate();
    final agentPubkey = 'A' * 64;
    final send = _sendMessage(session, nsec: signer.nsec);

    final result = send(
      channelId: _channelId,
      content: 'hello',
      isDirectMessage: true,
      directMessageRecipientPubkeys: [signer.public, agentPubkey],
    );
    await session.published;

    expect(
      session.event.tags,
      contains(equals(['p', agentPubkey.toLowerCase()])),
    );
    expect(
      session.event.tags.where((tag) => tag.isNotEmpty && tag[0] == 'p'),
      hasLength(1),
    );

    session.accept();
    await result;
  });

  test(
    'DM recipients and explicit mentions are normalized and deduplicated',
    () async {
      final session = _PendingPublishRelaySession();
      final signer = nostr.Keys.generate();
      final agentPubkey = 'B' * 64;
      final thirdPubkey = 'c' * 64;
      final send = _sendMessage(session, nsec: signer.nsec);

      final result = send(
        channelId: _channelId,
        content: 'hello',
        mentionPubkeys: [agentPubkey.toLowerCase(), thirdPubkey],
        isDirectMessage: true,
        directMessageRecipientPubkeys: [signer.public, agentPubkey],
      );
      await session.published;

      expect(
        session.event.tags.where((tag) => tag.isNotEmpty && tag[0] == 'p'),
        [
          ['p', agentPubkey.toLowerCase()],
          ['p', thirdPubkey],
        ],
      );

      session.accept();
      await result;
    },
  );

  test('plain stream messages preserve explicit-mention semantics', () async {
    final session = _PendingPublishRelaySession();
    final signer = nostr.Keys.generate();
    final send = _sendMessage(session, nsec: signer.nsec);

    final result = send(
      channelId: _channelId,
      content: 'hello',
      directMessageRecipientPubkeys: ['d' * 64],
    );
    await session.published;

    expect(
      session.event.tags.where((tag) => tag.isNotEmpty && tag[0] == 'p'),
      isEmpty,
    );

    session.accept();
    await result;
  });

  test('DM thread replies preserve recipient and reply tags', () async {
    final session = _PendingPublishRelaySession();
    final signer = nostr.Keys.generate();
    final agentPubkey = 'd' * 64;
    final send = _sendMessage(session, nsec: signer.nsec);

    final result = send(
      channelId: _channelId,
      content: 'thread reply',
      parentEventId: 'parent-id',
      rootEventId: 'root-id',
      isDirectMessage: true,
      directMessageRecipientPubkeys: [signer.public, agentPubkey],
    );
    await session.published;

    expect(session.event.tags, contains(equals(['e', 'root-id', '', 'root'])));
    expect(
      session.event.tags,
      contains(equals(['e', 'parent-id', '', 'reply'])),
    );
    expect(session.event.tags, contains(equals(['p', agentPubkey])));

    session.accept();
    await result;
  });

  test(
    'DM send falls back to channel members when participants are uncached',
    () async {
      final session = _PendingPublishRelaySession();
      final signer = nostr.Keys.generate();
      final agentPubkey = 'e' * 64;
      final send = _sendMessage(
        session,
        nsec: signer.nsec,
        resolveIsDirectMessage: (_) async => true,
        members: [
          ChannelMember(
            pubkey: signer.public,
            role: 'member',
            joinedAt: DateTime(2026),
          ),
          ChannelMember(
            pubkey: agentPubkey,
            role: 'bot',
            joinedAt: DateTime(2026),
          ),
        ],
      );

      final result = send(channelId: _channelId, content: 'hello');
      await session.published;

      expect(session.event.tags, contains(equals(['p', agentPubkey])));

      session.accept();
      await result;
    },
  );

  test('cancels delivery after the active community changes', () async {
    final container = ProviderContainer();
    addTearDown(container.dispose);
    container
        .read(relayConfigProvider.notifier)
        .update(baseUrl: 'https://first.example');
    final send = container.read(sendMessageProvider);

    container
        .read(relayConfigProvider.notifier)
        .update(baseUrl: 'https://second.example');

    await expectLater(
      send(channelId: _channelId, content: 'old community draft'),
      throwsA(
        isA<StateError>().having(
          (error) => error.message,
          'message',
          contains('active community changed'),
        ),
      ),
    );
  });
}

const _channelId = '11111111-1111-4111-8111-111111111111';

SendMessage _sendMessage(
  RelaySessionNotifier session, {
  required String nsec,
  List<ChannelMember> members = const [],
  Future<bool> Function(String channelId)? resolveIsDirectMessage,
}) => SendMessage(
  signedEventRelay: SignedEventRelay(session: session, nsec: nsec),
  fetchMembers: (_) async => members,
  readUserCache: () => const {},
  addLocalMessage: (_, _) {},
  completeLocalMessage: (_, _) {},
  removeLocalMessage: (_, _) {},
  resolveIsDirectMessage: resolveIsDirectMessage,
);

class _PendingPublishRelaySession extends RelaySessionNotifier {
  final Completer<NostrEvent> _result = Completer<NostrEvent>();
  final Completer<void> _published = Completer<void>();
  late NostrEvent event;

  Future<void> get published => _published.future;

  @override
  SessionState build() => const SessionState(status: SessionStatus.connected);

  @override
  Future<NostrEvent> publish(
    NostrEvent event, {
    Duration timeout = const Duration(seconds: 8),
  }) {
    this.event = event;
    _published.complete();
    return _result.future;
  }

  void accept() => _result.complete(event);

  void reject() => _result.completeError(Exception('relay rejected event'));
}
