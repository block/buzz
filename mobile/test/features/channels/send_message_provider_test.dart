import 'dart:async';

import 'package:buzz/features/channels/channel.dart';
import 'package:buzz/features/channels/channel_management_provider.dart';
import 'package:buzz/features/channels/send_message_provider.dart';
import 'package:buzz/shared/relay/relay.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:nostr/nostr.dart' as nostr;

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
        fetchChannel: (_) async => null,
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
      fetchChannel: (_) async => null,
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
    final keys = nostr.Keys.generate();
    final agentPubkey = nostr.Keys.generate().public;
    final session = _PendingPublishRelaySession();
    final send = SendMessage(
      signedEventRelay: SignedEventRelay(session: session, nsec: keys.nsec),
      fetchMembers: (_) async => [
        ChannelMember(
          pubkey: keys.public,
          role: 'member',
          joinedAt: DateTime.utc(2026),
        ),
        ChannelMember(
          pubkey: agentPubkey,
          role: 'bot',
          joinedAt: DateTime.utc(2026),
        ),
      ],
      fetchChannel: (_) async => Channel(
        id: _dmChannelId,
        name: 'DM',
        channelType: 'dm',
        visibility: 'private',
        description: '',
        createdBy: keys.public,
        createdAt: DateTime.utc(2026),
        memberCount: 2,
        participantPubkeys: [keys.public, agentPubkey],
        isMember: true,
      ),
      readUserCache: () => const {},
      addLocalMessage: (_, _) {},
      completeLocalMessage: (_, _) {},
      removeLocalMessage: (_, _) {},
    );

    final result = send(
      channelId: _dmChannelId,
      content: 'hello',
      mentionPubkeys: const [],
    );
    await session.published;
    session.accept();
    await result;

    final pTags = [
      for (final tag in session.event.tags)
        if (tag.isNotEmpty && tag.first == 'p') tag[1].toLowerCase(),
    ];
    expect(pTags, [agentPubkey.toLowerCase()]);
  });

  test('stream messages do not invent DM recipient p-tags', () async {
    final keys = nostr.Keys.generate();
    final otherPubkey = nostr.Keys.generate().public;
    final session = _PendingPublishRelaySession();
    final send = SendMessage(
      signedEventRelay: SignedEventRelay(session: session, nsec: keys.nsec),
      fetchMembers: (_) async => [
        ChannelMember(
          pubkey: otherPubkey,
          role: 'member',
          joinedAt: DateTime.utc(2026),
        ),
      ],
      fetchChannel: (_) async => Channel(
        id: _channelId,
        name: 'general',
        channelType: 'stream',
        visibility: 'open',
        description: '',
        createdBy: keys.public,
        createdAt: DateTime.utc(2026),
        memberCount: 2,
        participantPubkeys: [otherPubkey],
        isMember: true,
      ),
      readUserCache: () => const {},
      addLocalMessage: (_, _) {},
      completeLocalMessage: (_, _) {},
      removeLocalMessage: (_, _) {},
    );

    final result = send(
      channelId: _channelId,
      content: 'hello',
      mentionPubkeys: const [],
    );
    await session.published;
    session.accept();
    await result;

    expect(
      session.event.tags.any((tag) => tag.isNotEmpty && tag.first == 'p'),
      isFalse,
    );
  });
}

const _channelId = '11111111-1111-4111-8111-111111111111';
const _dmChannelId = '22222222-2222-4222-8222-222222222222';

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
