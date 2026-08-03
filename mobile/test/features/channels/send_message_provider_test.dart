import 'dart:async';

import 'package:flutter_test/flutter_test.dart';
import 'package:nostr/nostr.dart' as nostr;
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
        fetchImplicitRecipients: (_) async => const [],
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
      fetchImplicitRecipients: (_) async => const [],
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
    'adds DM participants as implicit recipients without @mentions',
    () async {
      final keys = nostr.Keys.generate();
      final session = _PendingPublishRelaySession();
      final send = SendMessage(
        signedEventRelay: SignedEventRelay(session: session, nsec: keys.nsec),
        fetchMembers: (_) async => const [],
        fetchImplicitRecipients: (_) async => [
          keys.public.toUpperCase(),
          _agentPubkey.toUpperCase(),
          _agentPubkey,
          _humanPubkey,
        ],
        readUserCache: () => const {},
        addLocalMessage: (_, _) {},
        completeLocalMessage: (_, _) {},
        removeLocalMessage: (_, _) {},
      );

      final result = send(channelId: _channelId, content: 'status?');
      await session.published;

      expect(_pTags(session.event), [_agentPubkey, _humanPubkey]);

      session.accept();
      await result;
    },
  );

  test('merges explicit mentions with implicit DM recipients', () async {
    final session = _PendingPublishRelaySession();
    final send = SendMessage(
      signedEventRelay: SignedEventRelay(
        session: session,
        nsec: nostr.Keys.generate().nsec,
      ),
      fetchMembers: (_) async => const [],
      fetchImplicitRecipients: (_) async => const [_agentPubkey],
      readUserCache: () => const {},
      addLocalMessage: (_, _) {},
      completeLocalMessage: (_, _) {},
      removeLocalMessage: (_, _) {},
    );

    final result = send(
      channelId: _channelId,
      content: 'hello',
      mentionPubkeys: const [_humanPubkey, _agentPubkey],
    );
    await session.published;

    expect(_pTags(session.event), [_humanPubkey, _agentPubkey]);

    session.accept();
    await result;
  });
}

const _channelId = '11111111-1111-4111-8111-111111111111';
const _agentPubkey =
    'aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa';
const _humanPubkey =
    'bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb';

List<String> _pTags(NostrEvent event) => [
  for (final tag in event.tags)
    if (tag.length >= 2 && tag[0] == 'p') tag[1],
];

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
