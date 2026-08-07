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
        isDirectMessage: (_) async => false,
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
      isDirectMessage: (_) async => false,
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

  test('plain DM messages p-tag every participant except the sender', () async {
    final session = _PendingPublishRelaySession();
    final senderKeys = nostr.Keys.generate();
    final agentPubkey = nostr.Keys.generate().public;
    final humanPubkey = nostr.Keys.generate().public;
    final send = SendMessage(
      signedEventRelay: SignedEventRelay(
        session: session,
        nsec: senderKeys.nsec,
      ),
      fetchMembers: (_) async => [
        ChannelMember(
          pubkey: senderKeys.public,
          role: 'owner',
          joinedAt: DateTime.utc(2026),
        ),
        ChannelMember(
          pubkey: agentPubkey.toUpperCase(),
          role: 'bot',
          joinedAt: DateTime.utc(2026),
        ),
        ChannelMember(
          pubkey: humanPubkey,
          role: 'member',
          joinedAt: DateTime.utc(2026),
        ),
      ],
      isDirectMessage: (_) async => true,
      readUserCache: () => const {},
      addLocalMessage: (_, _) {},
      completeLocalMessage: (_, _) {},
      removeLocalMessage: (_, _) {},
    );

    final result = send(channelId: _channelId, content: 'hello');
    await session.published;

    final pTags = session.event.tags
        .where((tag) => tag.length >= 2 && tag[0] == 'p')
        .map((tag) => tag[1])
        .toList();
    expect(pTags, [agentPubkey, humanPubkey]);

    session.accept();
    await result;
  });

  test('DM thread replies p-tag every participant', () async {
    final session = _PendingPublishRelaySession();
    final senderKeys = nostr.Keys.generate();
    final recipientPubkey = nostr.Keys.generate().public;
    final send = SendMessage(
      signedEventRelay: SignedEventRelay(
        session: session,
        nsec: senderKeys.nsec,
      ),
      fetchMembers: (_) async => [
        ChannelMember(
          pubkey: senderKeys.public,
          role: 'owner',
          joinedAt: DateTime.utc(2026),
        ),
        ChannelMember(
          pubkey: recipientPubkey,
          role: 'bot',
          joinedAt: DateTime.utc(2026),
        ),
      ],
      isDirectMessage: (_) async => true,
      readUserCache: () => const {},
      addLocalMessage: (_, _) {},
      completeLocalMessage: (_, _) {},
      removeLocalMessage: (_, _) {},
    );

    final result = send(
      channelId: _channelId,
      content: 'thread reply',
      parentEventId: 'a' * 64,
      rootEventId: 'b' * 64,
    );
    await session.published;

    expect(session.event.tags.where((tag) => tag.firstOrNull == 'p').toList(), [
      ['p', recipientPubkey],
    ]);

    session.accept();
    await result;
  });

  test('stream messages preserve explicit-mention behavior', () async {
    final session = _PendingPublishRelaySession();
    final recipientPubkey = nostr.Keys.generate().public;
    var fetchedMembers = false;
    final send = SendMessage(
      signedEventRelay: SignedEventRelay(
        session: session,
        nsec: nostr.Keys.generate().nsec,
      ),
      fetchMembers: (_) async {
        fetchedMembers = true;
        return const [];
      },
      isDirectMessage: (_) async => false,
      readUserCache: () => const {},
      addLocalMessage: (_, _) {},
      completeLocalMessage: (_, _) {},
      removeLocalMessage: (_, _) {},
    );

    final result = send(
      channelId: _channelId,
      content: 'hello agent',
      mentionPubkeys: [recipientPubkey.toUpperCase(), recipientPubkey],
    );
    await session.published;

    expect(fetchedMembers, isFalse);
    expect(session.event.tags.where((tag) => tag.firstOrNull == 'p').toList(), [
      ['p', recipientPubkey],
    ]);

    session.accept();
    await result;
  });

  test('fails closed when the channel type cannot be determined', () async {
    final session = _PendingPublishRelaySession();
    var fetchedMembers = false;
    var addedLocalMessage = false;
    final send = SendMessage(
      signedEventRelay: SignedEventRelay(
        session: session,
        nsec: nostr.Keys.generate().nsec,
      ),
      fetchMembers: (_) async {
        fetchedMembers = true;
        return const [];
      },
      isDirectMessage: (_) async => throw StateError('unknown channel'),
      readUserCache: () => const {},
      addLocalMessage: (_, _) => addedLocalMessage = true,
      completeLocalMessage: (_, _) {},
      removeLocalMessage: (_, _) {},
    );

    await expectLater(
      send(channelId: _channelId, content: 'hello'),
      throwsA(isA<StateError>()),
    );

    expect(fetchedMembers, isFalse);
    expect(addedLocalMessage, isFalse);
  });
}

const _channelId = '11111111-1111-4111-8111-111111111111';

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
