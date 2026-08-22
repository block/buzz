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

  test('a thread reply p-tags the author it answers', () async {
    // Agent harnesses subscribe with `#p`, so a reply without this tag is
    // never delivered to the agent being replied to.
    final session = _PendingPublishRelaySession();
    final send = SendMessage(
      signedEventRelay: SignedEventRelay(
        session: session,
        nsec: nostr.Keys.generate().nsec,
      ),
      fetchMembers: (_) async => const [],
      readUserCache: () => const {},
      addLocalMessage: (_, _) {},
      completeLocalMessage: (_, _) {},
      removeLocalMessage: (_, _) {},
    );

    final result = send(
      channelId: _channelId,
      content: 'thanks',
      parentEventId: _parentEventId,
      parentAuthorPubkey: _agentPubkey,
    );
    await session.published;
    session.accept();
    await result;

    // `containsAll` with matchers: Dart lists compare by identity, so a bare
    // `contains(['p', ...])` never matches an equal-but-distinct list.
    expect(
      session.event.tags,
      containsAll([
        // Marked, not bare: a bare `p` tag here is byte-identical to a typed
        // @mention, which forces the receiver to fetch the parent just to tell
        // them apart. Relay tag filters match only the second element, so the
        // marker cannot affect the agent's `#p` delivery.
        equals(['p', _agentPubkey, '', 'reply']),
        equals(['e', _parentEventId, '', 'reply']),
      ]),
    );
  });

  test(
    'a DM reply marks the counterpart as addressing, not as a mention',
    () async {
      // The counterpart is both the DM's other participant and the author being
      // answered. One tag, marked `reply` — marking it `mention` would claim they
      // had been typed as `@name`, which pierces a mute and outranks a real
      // `@you` in the mention feed.
      final session = _PendingPublishRelaySession();
      final signingKey = nostr.Keys.generate().nsec;
      final sender = nostr.Keys(
        nostr.Nip19.decode(payload: signingKey).data,
      ).public;
      final counterpart = 'c' * 64;
      final send = SendMessage(
        signedEventRelay: SignedEventRelay(session: session, nsec: signingKey),
        fetchMembers: (_) async => [_member(sender), _member(counterpart)],
        readUserCache: () => const {},
        addLocalMessage: (_, _) {},
        completeLocalMessage: (_, _) {},
        removeLocalMessage: (_, _) {},
      );

      final result = send(
        channelId: _channelId,
        content: 'thanks',
        channel: _dmChannel([sender, counterpart]),
        parentEventId: _parentEventId,
        parentAuthorPubkey: counterpart,
        mentionPubkeys: const [],
      );
      await session.published;

      expect(session.event.tags.where((tag) => tag.first == 'p').toList(), [
        ['p', counterpart, '', 'reply'],
      ]);

      session.accept();
      await result;
    },
  );

  test('a DM reply to your own message leaves the counterpart bare', () async {
    // Nobody typed the counterpart's name and they did not write the parent, so
    // neither marker is true of them. Bare means "ask the parent", which is the
    // answer they had before markers existed.
    final session = _PendingPublishRelaySession();
    final signingKey = nostr.Keys.generate().nsec;
    final sender = nostr.Keys(
      nostr.Nip19.decode(payload: signingKey).data,
    ).public;
    final counterpart = 'c' * 64;
    final send = SendMessage(
      signedEventRelay: SignedEventRelay(session: session, nsec: signingKey),
      fetchMembers: (_) async => [_member(sender), _member(counterpart)],
      readUserCache: () => const {},
      addLocalMessage: (_, _) {},
      completeLocalMessage: (_, _) {},
      removeLocalMessage: (_, _) {},
    );

    final result = send(
      channelId: _channelId,
      content: 'following up on my own note',
      channel: _dmChannel([sender, counterpart]),
      parentEventId: _parentEventId,
      parentAuthorPubkey: sender,
      mentionPubkeys: const [],
    );
    await session.published;

    expect(session.event.tags.where((tag) => tag.first == 'p').toList(), [
      ['p', counterpart],
    ]);

    session.accept();
    await result;
  });

  test('a reply marks a typed mention apart from the author it answers', () async {
    // The two roles are byte-identical as bare `p` tags, and this is the case
    // the receiver cannot resolve by fetching the parent: one of these pubkeys
    // was typed in the body and the other wrote the message being answered.
    final session = _PendingPublishRelaySession();
    final typed = 'd' * 64;
    final send = SendMessage(
      signedEventRelay: SignedEventRelay(
        session: session,
        nsec: nostr.Keys.generate().nsec,
      ),
      fetchMembers: (_) async => const [],
      readUserCache: () => const {},
      addLocalMessage: (_, _) {},
      completeLocalMessage: (_, _) {},
      removeLocalMessage: (_, _) {},
    );

    final result = send(
      channelId: _channelId,
      content: 'thanks @typed',
      parentEventId: _parentEventId,
      parentAuthorPubkey: _agentPubkey,
      mentionPubkeys: [typed],
    );
    await session.published;

    // Mentions first, addressing last — the order the Rust builder emits, so an
    // optimistic copy of this event is tag-identical to what the relay stores.
    expect(session.event.tags.where((tag) => tag.first == 'p').toList(), [
      ['p', typed, '', 'mention'],
      ['p', _agentPubkey, '', 'reply'],
    ]);

    session.accept();
    await result;
  });

  test('a typed parent author is tagged once, as a mention', () async {
    // Mention outranks addressing: typing someone's name is a stronger claim
    // than answering them, and two tags for one pubkey would let the reply
    // count twice.
    final session = _PendingPublishRelaySession();
    final send = SendMessage(
      signedEventRelay: SignedEventRelay(
        session: session,
        nsec: nostr.Keys.generate().nsec,
      ),
      fetchMembers: (_) async => const [],
      readUserCache: () => const {},
      addLocalMessage: (_, _) {},
      completeLocalMessage: (_, _) {},
      removeLocalMessage: (_, _) {},
    );

    final result = send(
      channelId: _channelId,
      content: 'thanks @agent',
      parentEventId: _parentEventId,
      parentAuthorPubkey: _agentPubkey,
      mentionPubkeys: const [_agentPubkey],
    );
    await session.published;

    expect(session.event.tags.where((tag) => tag.first == 'p').toList(), [
      ['p', _agentPubkey, '', 'mention'],
    ]);

    session.accept();
    await result;
  });

  test('a malformed parent author adds no addressing tag', () async {
    // A `p` tag is what delivers the reply to the agent being answered, so a
    // malformed value there would be published as a tag naming nobody.
    final session = _PendingPublishRelaySession();
    final send = SendMessage(
      signedEventRelay: SignedEventRelay(
        session: session,
        nsec: nostr.Keys.generate().nsec,
      ),
      fetchMembers: (_) async => const [],
      readUserCache: () => const {},
      addLocalMessage: (_, _) {},
      completeLocalMessage: (_, _) {},
      removeLocalMessage: (_, _) {},
    );

    final result = send(
      channelId: _channelId,
      content: 'thanks',
      parentEventId: _parentEventId,
      parentAuthorPubkey: 'not-a-pubkey',
      mentionPubkeys: const [],
    );
    await session.published;

    expect(session.event.tags.where((tag) => tag.first == 'p'), isEmpty);

    session.accept();
    await result;
  });
}

const _channelId = '11111111-1111-4111-8111-111111111111';
const _parentEventId =
    'fdfdfdfdfdfdfdfdfdfdfdfdfdfdfdfdfdfdfdfdfdfdfdfdfdfdfdfdfdfdfdfd';
const _agentPubkey =
    'aeaeaeaeaeaeaeaeaeaeaeaeaeaeaeaeaeaeaeaeaeaeaeaeaeaeaeaeaeaeaeae';

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
