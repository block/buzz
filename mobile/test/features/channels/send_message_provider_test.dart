import 'dart:async';

import 'package:flutter_test/flutter_test.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';
import 'package:nostr/nostr.dart' as nostr;
import 'package:buzz/features/channels/channel.dart';
import 'package:buzz/features/channels/channel_management_provider.dart';
import 'package:buzz/features/channels/send_message_provider.dart';
import 'package:buzz/shared/profile/user_profile.dart';
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

  test(
    'resolves a plain-text @mention never tapped in the picker (non-DM channel)',
    () async {
      // Reproduces #7449: the compose bar always passes a concrete list to
      // `mentionPubkeys` (never null), even when the user typed "@Name"
      // without selecting it from the picker. That silently suppressed the
      // text-resolution fallback.
      final session = _PendingPublishRelaySession();
      final signingKey = nostr.Keys.generate().nsec;
      final sender = nostr.Keys(
        nostr.Nip19.decode(payload: signingKey).data,
      ).public;
      final alice = 'n' * 64;
      final send = SendMessage(
        signedEventRelay: SignedEventRelay(session: session, nsec: signingKey),
        fetchMembers: (_) async => [_member(sender), _member(alice)],
        readUserCache: () => {
          alice: UserProfile(pubkey: alice, displayName: 'Alice'),
        },
        addLocalMessage: (_, _) {},
        completeLocalMessage: (_, _) {},
        removeLocalMessage: (_, _) {},
      );

      final result = send(
        channelId: _channelId,
        content: '@Alice hi',
        // Exactly what the compose bar sends today when nothing was
        // tapped in the mention picker: empty, not null.
        mentionPubkeys: const [],
      );
      await session.published;

      expect(session.event.tags.where((tag) => tag.first == 'p').toList(), [
        ['p', alice],
      ]);

      session.accept();
      await result;
    },
  );

  test(
    'does not duplicate a pubkey tapped in the picker AND present as text',
    () async {
      final session = _PendingPublishRelaySession();
      final signingKey = nostr.Keys.generate().nsec;
      final sender = nostr.Keys(
        nostr.Nip19.decode(payload: signingKey).data,
      ).public;
      final alice = 'n' * 64;
      final send = SendMessage(
        signedEventRelay: SignedEventRelay(session: session, nsec: signingKey),
        fetchMembers: (_) async => [_member(sender), _member(alice)],
        readUserCache: () => {
          alice: UserProfile(pubkey: alice, displayName: 'Alice'),
        },
        addLocalMessage: (_, _) {},
        completeLocalMessage: (_, _) {},
        removeLocalMessage: (_, _) {},
      );

      // The picker already resolved "@Alice" to a tap-selected pubkey; the
      // same name is also present as text. The union must not double-tag.
      final result = send(
        channelId: _channelId,
        content: '@Alice hi',
        mentionPubkeys: [alice],
      );
      await session.published;

      expect(session.event.tags.where((tag) => tag.first == 'p').toList(), [
        ['p', alice],
      ]);

      session.accept();
      await result;
    },
  );

  test(
    'preserves a picker-tapped recipient the text resolver cannot see',
    () async {
      final session = _PendingPublishRelaySession();
      final signingKey = nostr.Keys.generate().nsec;
      final sender = nostr.Keys(
        nostr.Nip19.decode(payload: signingKey).data,
      ).public;
      final alice = 'n' * 64;
      final send = SendMessage(
        signedEventRelay: SignedEventRelay(session: session, nsec: signingKey),
        fetchMembers: (_) async => [_member(sender), _member(alice)],
        // Empty cache: the text resolver has nothing to match against, so
        // it would contribute zero pubkeys on its own. The picker's tap
        // must still make it through untouched.
        readUserCache: () => const {},
        addLocalMessage: (_, _) {},
        completeLocalMessage: (_, _) {},
        removeLocalMessage: (_, _) {},
      );

      final result = send(
        channelId: _channelId,
        content: '@Alice hi',
        mentionPubkeys: [alice],
      );
      await session.published;

      expect(session.event.tags.where((tag) => tag.first == 'p').toList(), [
        ['p', alice],
      ]);

      session.accept();
      await result;
    },
  );

  test('an ambiguous name shared by two members notifies neither via text '
      'resolution', () async {
    final session = _PendingPublishRelaySession();
    final signingKey = nostr.Keys.generate().nsec;
    final sender = nostr.Keys(
      nostr.Nip19.decode(payload: signingKey).data,
    ).public;
    final aliceA = 'a' * 64;
    final aliceB = 'b' * 64;
    final send = SendMessage(
      signedEventRelay: SignedEventRelay(session: session, nsec: signingKey),
      fetchMembers: (_) async => [
        _member(sender),
        _member(aliceA),
        _member(aliceB),
      ],
      readUserCache: () => {
        aliceA: UserProfile(pubkey: aliceA, displayName: 'Alice'),
        aliceB: UserProfile(pubkey: aliceB, displayName: 'Alice'),
      },
      addLocalMessage: (_, _) {},
      completeLocalMessage: (_, _) {},
      removeLocalMessage: (_, _) {},
    );

    // Two distinct members named "Alice" in the same channel: resolving
    // to either one would be a guess, and guessing wrong pings the
    // wrong agent/person. Neither gets tagged.
    final result = send(
      channelId: _channelId,
      content: '@Alice hi',
      mentionPubkeys: const [],
    );
    await session.published;

    expect(session.event.tags.where((tag) => tag.first == 'p').toList(), []);

    session.accept();
    await result;
  });

  test('ignores an @name that only appears inside a code span or a quoted '
      'line', () async {
    final session = _PendingPublishRelaySession();
    final signingKey = nostr.Keys.generate().nsec;
    final sender = nostr.Keys(
      nostr.Nip19.decode(payload: signingKey).data,
    ).public;
    final alice = 'n' * 64;
    final send = SendMessage(
      signedEventRelay: SignedEventRelay(session: session, nsec: signingKey),
      fetchMembers: (_) async => [_member(sender), _member(alice)],
      readUserCache: () => {
        alice: UserProfile(pubkey: alice, displayName: 'Alice'),
      },
      addLocalMessage: (_, _) {},
      completeLocalMessage: (_, _) {},
      removeLocalMessage: (_, _) {},
    );

    final result = send(
      channelId: _channelId,
      content:
          'sample: `@Alice hi` and\n> quoting @Alice from before\nno real mention here',
      mentionPubkeys: const [],
    );
    await session.published;

    expect(session.event.tags.where((tag) => tag.first == 'p').toList(), []);

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
