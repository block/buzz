import 'dart:async';

import 'package:flutter_test/flutter_test.dart';
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
        isDmChannel: (_) => false,
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
      isDmChannel: (_) => false,
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
    'addresses every DM participant so agent subscribers receive it',
    () async {
      final session = _PendingPublishRelaySession();
      final signer = nostr.Keys.generate();
      final agent = nostr.Keys.generate();
      final other = nostr.Keys.generate();
      final send = SendMessage(
        signedEventRelay: SignedEventRelay(
          session: session,
          nsec: signer.nsec,
        ),
        fetchMembers: (_) async => [
          ChannelMember(
            pubkey: signer.public,
            role: 'member',
            joinedAt: DateTime.fromMillisecondsSinceEpoch(0),
          ),
          ChannelMember(
            pubkey: agent.public,
            role: 'bot',
            joinedAt: DateTime.fromMillisecondsSinceEpoch(0),
          ),
          ChannelMember(
            pubkey: other.public,
            role: 'member',
            joinedAt: DateTime.fromMillisecondsSinceEpoch(0),
          ),
        ],
        isDmChannel: (_) => true,
        readUserCache: () => const {},
        addLocalMessage: (_, _) {},
        completeLocalMessage: (_, _) {},
        removeLocalMessage: (_, _) {},
      );

      final result = send(channelId: _channelId, content: 'hello');
      await session.published;

      expect(session.event.tags, containsAll([
        ['p', agent.public],
        ['p', other.public],
      ]));
      expect(session.event.tags, isNot(contains(['p', signer.public])));

      session.accept();
      await result;
    },
  );
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
