import 'dart:async';

import 'package:flutter_test/flutter_test.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';
import 'package:nostr/nostr.dart' as nostr;
import 'package:buzz/features/channels/channel.dart';
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
      final animatedIds = <String>[];
      final send = SendMessage(
        signedEventRelay: SignedEventRelay(
          session: session,
          nsec: nostr.Keys.generate().nsec,
        ),
        fetchMembers: (_) async => const [],
        readUserCache: () => const {},
        addLocalMessage: (_, event) => localMessages.add(event),
        markLocalMessageForAnimation: (_, eventId) => animatedIds.add(eventId),
        completeLocalMessage: (_, eventId) => completedIds.add(eventId),
        removeLocalMessage: (_, eventId) => removedIds.add(eventId),
      );

      final result = send(channelId: _channelId, content: 'hello');
      await session.published;

      expect(localMessages, hasLength(1));
      expect(localMessages.single.id, session.event.id);
      expect(localMessages.single.content, 'hello');
      expect(localMessages.single.channelId, _channelId);
      expect(animatedIds, [localMessages.single.id]);
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

  test(
    'keeps a failed message for retry, then resends the same signed event',
    () async {
      final session = _ScriptedPublishRelaySession();
      final pending = <String, NostrEvent>{};
      final failedIds = <String>[];
      final completedIds = <String>[];
      final removedIds = <String>[];
      final send = SendMessage(
        signedEventRelay: SignedEventRelay(
          session: session,
          nsec: nostr.Keys.generate().nsec,
        ),
        fetchMembers: (_) async => const [],
        readUserCache: () => const {},
        addLocalMessage: (_, event) => pending[event.id] = event,
        completeLocalMessage: (_, eventId) {
          pending.remove(eventId);
          completedIds.add(eventId);
        },
        removeLocalMessage: (_, eventId) {
          pending.remove(eventId);
          removedIds.add(eventId);
        },
        readPendingLocalMessage: (_, eventId) => pending[eventId],
        failLocalMessage: (_, eventId) => failedIds.add(eventId),
        publishSigned: session.publish,
      );

      // Offline: the bubble stays, marked failed, and the draft is not
      // handed back to the composer as an error.
      session.outcomes.add(StateError('Relay session is not connected'));
      await send(channelId: _channelId, content: 'hello');
      final id = session.published.single.id;
      expect(failedIds, [id]);
      expect(pending.keys, [id]);
      expect(removedIds, isEmpty);

      // A second failure keeps it failed.
      session.outcomes.add(TimeoutException('no ack'));
      await send.retry(channelId: _channelId, eventId: id);
      expect(failedIds, [id, id]);

      // Retry republishes the exact signed bytes.
      session.outcomes.add(null);
      await send.retry(channelId: _channelId, eventId: id);
      expect(session.published.map((event) => event.id), [id, id, id]);
      expect(completedIds, [id]);
      expect(pending, isEmpty);

      // An old failure is past the relay's clock window, so it is signed
      // again as a new message that replaces the failed bubble.
      session.outcomes.add(StateError('Relay session is not connected'));
      await send(channelId: _channelId, content: 'stale');
      final staleId = session.published.last.id;
      final staleEvent = pending[staleId]!;
      pending[staleId] = NostrEvent.fromJson({
        ...staleEvent.toJson(),
        'created_at': staleEvent.createdAt - 3600,
      });
      session.outcomes.add(null);
      await send.retry(channelId: _channelId, eventId: staleId);
      final resent = session.published.last;
      expect(resent.createdAt, staleEvent.createdAt, reason: 'signed now');
      expect(resent.content, 'stale');
      expect(resent.tags, staleEvent.tags);
      expect(removedIds, [staleId]);
      expect(completedIds.last, resent.id);
      expect(pending, isEmpty);

      // Retrying a message that was delivered meanwhile does nothing.
      await send.retry(channelId: _channelId, eventId: resent.id);
      expect(session.published.last.id, resent.id);

      // Delete discards a failed message instead.
      session.outcomes.add(StateError('Relay session is not connected'));
      await send(channelId: _channelId, content: 'second');
      final second = session.published.last.id;
      send.discard(channelId: _channelId, eventId: second);
      expect(removedIds, [staleId, second]);
      expect(pending, isEmpty);
    },
  );

  test('final signed event addresses the current DM agent member', () async {
    final session = _PendingPublishRelaySession();
    final signingKey = nostr.Keys.generate().nsec;
    final sender = nostr.Keys(
      nostr.Nip19.decode(payload: signingKey).data,
    ).public;
    final staleAgent = 'a' * 64;
    final activeAgent = 'c' * 64;
    final human = 'b' * 64;
    final send = SendMessage(
      signedEventRelay: SignedEventRelay(session: session, nsec: signingKey),
      fetchMembers: (_) async => [
        _member(sender),
        _member(activeAgent),
        _member(human),
      ],
      readUserCache: () => const {},
      addLocalMessage: (_, _) {},
      completeLocalMessage: (_, _) {},
      removeLocalMessage: (_, _) {},
    );

    final result = send(
      channelId: _channelId,
      content: 'hello without a visible mention',
      // Metadata still names the replaced agent. Delivery must follow the
      // authoritative current membership snapshot instead.
      channel: _dmChannel([sender, staleAgent, human]),
      mentionPubkeys: const [],
    );
    await session.published;

    expect(session.event.content, 'hello without a visible mention');
    expect(session.event.tags.where((tag) => tag.first == 'p').toList(), [
      ['p', activeAgent],
      ['p', human],
    ]);

    session.accept();
    await result;
  });

  test('final signed event addresses a human DM recipient', () async {
    final session = _PendingPublishRelaySession();
    final signingKey = nostr.Keys.generate().nsec;
    final sender = nostr.Keys(
      nostr.Nip19.decode(payload: signingKey).data,
    ).public;
    final human = 'b' * 64;
    final send = SendMessage(
      signedEventRelay: SignedEventRelay(session: session, nsec: signingKey),
      fetchMembers: (_) async => [_member(sender), _member(human)],
      readUserCache: () => const {},
      addLocalMessage: (_, _) {},
      completeLocalMessage: (_, _) {},
      removeLocalMessage: (_, _) {},
    );

    final result = send(
      channelId: _channelId,
      content: 'hello human',
      channel: _dmChannel([sender, human]),
      mentionPubkeys: const [],
    );
    await session.published;

    expect(session.event.tags.where((tag) => tag.first == 'p').toList(), [
      ['p', human],
    ]);

    session.accept();
    await result;
  });

  test(
    'falls back to metadata DM recipients when membership is empty',
    () async {
      final session = _PendingPublishRelaySession();
      final signingKey = nostr.Keys.generate().nsec;
      final sender = nostr.Keys(
        nostr.Nip19.decode(payload: signingKey).data,
      ).public;
      final recipient = 'b' * 64;
      final send = SendMessage(
        signedEventRelay: SignedEventRelay(session: session, nsec: signingKey),
        fetchMembers: (_) async => const [],
        readUserCache: () => const {},
        addLocalMessage: (_, _) {},
        completeLocalMessage: (_, _) {},
        removeLocalMessage: (_, _) {},
      );

      final result = send(
        channelId: _channelId,
        content: 'hello from an unavailable roster',
        channel: _dmChannel([sender, recipient]),
        mentionPubkeys: const [],
      );
      await session.published;

      expect(session.event.tags.where((tag) => tag.first == 'p').toList(), [
        ['p', recipient],
      ]);

      session.accept();
      await result;
    },
  );

  test('falls back to metadata DM recipients when membership fails', () async {
    final session = _PendingPublishRelaySession();
    final signingKey = nostr.Keys.generate().nsec;
    final sender = nostr.Keys(
      nostr.Nip19.decode(payload: signingKey).data,
    ).public;
    final recipientOne = 'b' * 64;
    final recipientTwo = 'c' * 64;
    final send = SendMessage(
      signedEventRelay: SignedEventRelay(session: session, nsec: signingKey),
      fetchMembers: (_) async => throw StateError('membership unavailable'),
      readUserCache: () => const {},
      addLocalMessage: (_, _) {},
      completeLocalMessage: (_, _) {},
      removeLocalMessage: (_, _) {},
    );

    final result = send(
      channelId: _channelId,
      content: 'hello group',
      channel: _dmChannel([sender, recipientOne, recipientTwo]),
      mentionPubkeys: [recipientOne.toUpperCase()],
    );
    await session.published;

    expect(session.event.tags.where((tag) => tag.first == 'p').toList(), [
      ['p', recipientOne],
      ['p', recipientTwo],
    ]);

    session.accept();
    await result;
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
}

const _channelId = '11111111-1111-4111-8111-111111111111';

Channel _dmChannel(List<String> participantPubkeys) => Channel(
  id: _channelId,
  name: 'DM',
  channelType: 'dm',
  visibility: 'private',
  description: '',
  createdBy: participantPubkeys.first,
  createdAt: DateTime(2025),
  memberCount: participantPubkeys.length,
  participantPubkeys: participantPubkeys,
  isMember: true,
);

ChannelMember _member(String pubkey, {String role = 'member'}) =>
    ChannelMember(pubkey: pubkey, role: role, joinedAt: DateTime(2025));

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

class _ScriptedPublishRelaySession extends RelaySessionNotifier {
  final published = <NostrEvent>[];

  /// Each publish consumes one outcome: null accepts, an error rejects.
  final outcomes = <Object?>[];

  @override
  SessionState build() => const SessionState(status: SessionStatus.connected);

  @override
  Future<NostrEvent> publish(
    NostrEvent event, {
    Duration timeout = const Duration(seconds: 8),
  }) async {
    published.add(event);
    final outcome = outcomes.removeAt(0);
    if (outcome != null) throw outcome;
    return event;
  }
}
