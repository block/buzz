import 'dart:async';

import 'package:flutter_test/flutter_test.dart';
import 'package:nostr/nostr.dart' as nostr;
import 'package:buzz/features/channels/channel.dart';
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
        readChannel: (_) => _channel(channelType: 'stream'),
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
      readChannel: (_) => _channel(channelType: 'stream'),
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

  test('plain DM send p-tags every participant except the sender', () async {
    final session = _PendingPublishRelaySession();
    final keys = nostr.Keys.generate();
    final send = _sendMessage(
      session: session,
      nsec: keys.nsec,
      readChannel: (channelId) => _channel(
        channelType: 'dm',
        // Roster includes the sender's own pubkey — it must be excluded.
        participantPubkeys: [keys.public, _humanPubkey, _agentPubkey],
      ),
    );

    final result = send(
      channelId: _channelId,
      content: 'hey, can you look at this?',
      mentionPubkeys: const [],
    );
    await session.published;
    session.accept();
    await result;

    expect(_pTagPubkeys(session.event), [_humanPubkey, _agentPubkey]);
  });

  test('non-DM channel carries explicit mentions only', () async {
    final session = _PendingPublishRelaySession();
    final send = _sendMessage(
      session: session,
      nsec: nostr.Keys.generate().nsec,
      readChannel: (channelId) => _channel(
        channelType: 'stream',
        participantPubkeys: const [_humanPubkey, _agentPubkey],
      ),
    );

    final result = send(
      channelId: _channelId,
      content: 'hey @someone',
      mentionPubkeys: const [_mentionPubkey],
    );
    await session.published;
    session.accept();
    await result;

    expect(_pTagPubkeys(session.event), [_mentionPubkey]);
  });

  test(
    'DM recipient tags are lowercase, non-empty, and deduplicated',
    () async {
      final session = _PendingPublishRelaySession();
      final keys = nostr.Keys.generate();
      final send = _sendMessage(
        session: session,
        nsec: keys.nsec,
        readChannel: (channelId) => _channel(
          channelType: 'dm',
          participantPubkeys: [keys.public, _humanPubkey, _agentPubkey, ''],
        ),
      );

      final explicitAgent = _agentPubkey.toUpperCase();
      final result = send(
        channelId: _channelId,
        content: 'hey @agent',
        mentionPubkeys: [explicitAgent, ''],
      );
      await session.published;
      session.accept();
      await result;

      expect(_pTagPubkeys(session.event), [_agentPubkey, _humanPubkey]);
    },
  );

  test(
    'DM thread reply carries fan-out p tags alongside reply e-tags',
    () async {
      final session = _PendingPublishRelaySession();
      final keys = nostr.Keys.generate();
      final send = _sendMessage(
        session: session,
        nsec: keys.nsec,
        readChannel: (channelId) => _channel(
          channelType: 'dm',
          participantPubkeys: [keys.public, _humanPubkey, _agentPubkey],
        ),
      );

      // Nested thread reply, as the thread page sends it: parent + root.
      final result = send(
        channelId: _channelId,
        content: 'replying in-thread',
        parentEventId: _parentEventId,
        rootEventId: _rootEventId,
        mentionPubkeys: const [],
      );
      await session.published;
      session.accept();
      await result;

      expect(
        session.event.tags,
        containsAll([
          ['e', _rootEventId, '', 'root'],
          ['e', _parentEventId, '', 'reply'],
        ]),
      );
      expect(_pTagPubkeys(session.event), [_humanPubkey, _agentPubkey]);
    },
  );

  test('waits for channel loading before publishing a plain DM', () async {
    final session = _PendingPublishRelaySession();
    final channel = Completer<Channel?>();
    final send = _sendMessage(
      session: session,
      nsec: nostr.Keys.generate().nsec,
      readChannel: (_) => channel.future,
    );

    final result = send(
      channelId: _channelId,
      content: 'hello',
      mentionPubkeys: const [],
    );
    await Future<void>.delayed(Duration.zero);

    expect(session.hasPublished, isFalse);

    channel.complete(
      _channel(
        channelType: 'dm',
        participantPubkeys: const [_humanPubkey, _agentPubkey],
      ),
    );
    await session.published;
    session.accept();
    await result;

    expect(_pTagPubkeys(session.event), [_humanPubkey, _agentPubkey]);
  });
  test('rejects a send when the channel cannot be resolved', () async {
    final session = _PendingPublishRelaySession();
    final send = _sendMessage(
      session: session,
      nsec: nostr.Keys.generate().nsec,
      readChannel: (_) => null,
    );

    final result = send(
      channelId: _channelId,
      content: 'hello',
      mentionPubkeys: const [],
    );
    final failure = expectLater(
      result,
      throwsA(
        isA<StateError>().having(
          (error) => error.message,
          'message',
          contains(_channelId),
        ),
      ),
    );
    await Future<void>.delayed(Duration.zero);
    if (session.hasPublished) session.reject();

    await failure;
    expect(session.hasPublished, isFalse);
  });
}

const _channelId = '11111111-1111-4111-8111-111111111111';

const _humanPubkey =
    'aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa';
const _agentPubkey =
    'bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb';
const _mentionPubkey =
    'cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc';

const _rootEventId =
    'dddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddd';
const _parentEventId =
    'eeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee';

SendMessage _sendMessage({
  required _PendingPublishRelaySession session,
  required String nsec,
  required FutureOr<Channel?> Function(String channelId) readChannel,
}) => SendMessage(
  signedEventRelay: SignedEventRelay(session: session, nsec: nsec),
  readChannel: readChannel,
  fetchMembers: (_) async => const [],
  readUserCache: () => const {},
  addLocalMessage: (_, _) {},
  completeLocalMessage: (_, _) {},
  removeLocalMessage: (_, _) {},
);

Channel _channel({
  required String channelType,
  List<String> participantPubkeys = const [],
}) => Channel(
  id: _channelId,
  name: 'chat',
  channelType: channelType,
  visibility: 'private',
  description: '',
  createdBy: _humanPubkey,
  createdAt: DateTime.utc(2026),
  memberCount: participantPubkeys.length,
  participantPubkeys: participantPubkeys,
);

List<String> _pTagPubkeys(NostrEvent event) => [
  for (final tag in event.tags)
    if (tag.isNotEmpty && tag.first == 'p') tag[1],
];

class _PendingPublishRelaySession extends RelaySessionNotifier {
  final Completer<NostrEvent> _result = Completer<NostrEvent>();
  final Completer<void> _published = Completer<void>();
  late NostrEvent event;
  bool get hasPublished => _published.isCompleted;

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
