import 'dart:async';

import 'package:flutter_test/flutter_test.dart';
import 'package:nostr/nostr.dart' as nostr;
import 'package:buzz/features/channels/send_message_provider.dart';
import 'package:buzz/features/channels/channel_management_provider.dart';
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

  test(
    'tags every other DM participant without visible mention text',
    () async {
      final session = _PendingPublishRelaySession();
      final author = nostr.Keys.generate();
      final recipient = nostr.Keys.generate().public;
      final send = SendMessage(
        signedEventRelay: SignedEventRelay(session: session, nsec: author.nsec),
        fetchMembers: (_) async => [
          ChannelMember(
            pubkey: author.public,
            role: 'member',
            joinedAt: DateTime.utc(2026),
          ),
          ChannelMember(
            pubkey: recipient,
            role: 'member',
            joinedAt: DateTime.utc(2026),
          ),
        ],
        readUserCache: () => const {},
        addLocalMessage: (_, __) {},
        completeLocalMessage: (_, __) {},
        removeLocalMessage: (_, __) {},
      );

      final result = send(channelId: _channelId, content: 'hi', isDm: true);
      await session.published;

      expect(
        session.event.tags.any(
          (tag) => tag.length >= 2 && tag[0] == 'p' && tag[1] == recipient,
        ),
        isTrue,
      );
      expect(
        session.event.tags.any(
          (tag) => tag.length >= 2 && tag[0] == 'p' && tag[1] == author.public,
        ),
        isFalse,
      );
      session.accept();
      await result;
    },
  );

  test('keeps the DM recipient tag on thread replies', () async {
    final session = _PendingPublishRelaySession();
    final author = nostr.Keys.generate();
    final recipient = nostr.Keys.generate().public;
    final send = SendMessage(
      signedEventRelay: SignedEventRelay(session: session, nsec: author.nsec),
      fetchMembers: (_) async => [
        ChannelMember(
          pubkey: author.public,
          role: 'member',
          joinedAt: DateTime.utc(2026),
        ),
        ChannelMember(
          pubkey: recipient,
          role: 'bot',
          joinedAt: DateTime.utc(2026),
        ),
      ],
      readUserCache: () => const {},
      addLocalMessage: (_, __) {},
      completeLocalMessage: (_, __) {},
      removeLocalMessage: (_, __) {},
    );

    final result = send(
      channelId: _channelId,
      content: 'and in this thread?',
      parentEventId: 'parent-event-id',
      rootEventId: 'root-event-id',
      isDm: true,
    );
    await session.published;

    expect(
      session.event.tags.any(
        (tag) => tag.length >= 2 && tag[0] == 'p' && tag[1] == recipient,
      ),
      isTrue,
    );
    expect(
      session.event.tags.any(
        (tag) =>
            tag.length >= 4 &&
            tag[0] == 'e' &&
            tag[1] == 'root-event-id' &&
            tag[3] == 'root',
      ),
      isTrue,
    );
    expect(
      session.event.tags.any(
        (tag) =>
            tag.length >= 4 &&
            tag[0] == 'e' &&
            tag[1] == 'parent-event-id' &&
            tag[3] == 'reply',
      ),
      isTrue,
    );
    session.accept();
    await result;
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
