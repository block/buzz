import 'dart:async';

import 'package:flutter_test/flutter_test.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';
import 'package:nostr/nostr.dart' as nostr;
import 'package:buzz/features/channels/channel_management_provider.dart';
import 'package:buzz/features/channels/send_message_provider.dart';
import 'package:buzz/features/profile/user_profile.dart';
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

  test(
    'tags every candidate when a display name is ambiguous',
    () async {
      // Two channel members share the display name "Neuromancer": a stale
      // identity left behind by a re-key, and the live one. The stale entry is
      // deliberately first in the cache — resolution used to take the first
      // match and stop, so the `p` tag landed on the dead identity and the
      // real agent was never notified. Nothing surfaced to the sender.
      const staleP = 'f1a06b925f6705ef530932138664e2dda35fd5d743991f49e96';
      const liveP = '1667bcebe81f2e5af1e5561a385f1c8a91b84d8d19c5d9c3f1b';

      final session = _PendingPublishRelaySession();
      final send = SendMessage(
        signedEventRelay: SignedEventRelay(
          session: session,
          nsec: nostr.Keys.generate().nsec,
        ),
        fetchMembers: (_) async => [
          ChannelMember(
            pubkey: staleP,
            role: 'bot',
            joinedAt: DateTime.utc(2020),
          ),
          ChannelMember(
            pubkey: liveP,
            role: 'bot',
            joinedAt: DateTime.utc(2020),
          ),
        ],
        readUserCache: () => const {
          staleP: UserProfile(pubkey: staleP, displayName: 'Neuromancer'),
          liveP: UserProfile(pubkey: liveP, displayName: 'Neuromancer'),
        },
        addLocalMessage: (_, _) {},
        completeLocalMessage: (_, _) {},
        removeLocalMessage: (_, _) {},
      );

      final result = send(channelId: _channelId, content: '@Neuromancer hi');
      await session.published;
      session.accept();
      await result;

      final tagged = session.event.tags
          .where((t) => t.isNotEmpty && t.first == 'p')
          .map((t) => t[1])
          .toSet();

      // Both must be tagged; tagging only one is a coin flip on map ordering.
      expect(tagged, containsAll(<String>[staleP, liveP]));
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
