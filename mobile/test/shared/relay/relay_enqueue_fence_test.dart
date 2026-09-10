import 'package:buzz/features/channels/channel_management_provider.dart';
import 'package:buzz/shared/relay/relay.dart';
import 'package:buzz/features/channels/send_message_provider.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';
import 'package:nostr/nostr.dart' as nostr;

void main() {
  // Each row binds to the production object's own validity callback —
  // ChannelActions.isCommunityValid for kind:9000 and
  // SendMessage.isDeliveryValid for messages — exactly as
  // channelActionsProvider/sendMessageProvider wire them. There is no
  // test-authored outer guard here, so removing either production
  // `withRelayPublicationGuard` wrapper must fail its own rows.
  for (final kind in [9000, EventKind.streamMessage]) {
    for (final cancel in [false, true]) {
      test(
        'queued kind:$kind cancellation=$cancel via the owning guard',
        () async {
          final gate = RelayRateLimitGate();
          final session = RelaySessionNotifier(rateLimitGate: gate);
          final socket = _Socket();
          session.debugAttachSocketForTest(socket);
          final relay = SignedEventRelay(
            session: session,
            nsec: nostr.Keys.generate().nsec,
          );
          var valid = true;
          var accepted = 0;
          final container = ProviderContainer();
          addTearDown(container.dispose);
          final actions = container.read(
            Provider(
              (ref) => ChannelActions(
                ref: ref,
                session: session,
                signedEventRelay: relay,
                currentPubkey: relay.pubkey,
                isCommunityValid: () => valid,
              ),
            ),
          );
          gate.activate(10);
          // Real production signing/session/backpressure and ChannelActions for
          // kind9000. No query/selected-reader or publication recorder substitutes.
          final pending = kind == 9000
              ? actions.addMembers(
                  channelId: 'channel',
                  pubkeys: ['target'],
                  onAccepted: (_) => accepted++,
                )
              : SendMessage(
                  signedEventRelay: relay,
                  fetchMembers: (_) async => const [],
                  readUserCache: () => const {},
                  addLocalMessage: (_, _) {},
                  completeLocalMessage: (_, _) => accepted++,
                  removeLocalMessage: (_, _) {},
                  isDeliveryValid: () => valid,
                )(channelId: 'channel', content: '', mentionPubkeys: const []);
          Object? error;
          final settled = pending.then<void>(
            (_) {},
            onError: (Object e) {
              error = e;
            },
          );
          expect(socket.messages, isEmpty);
          // Invalidate the owning production object's own callback while the
          // real rate-limit gate still holds the publish.
          valid = !cancel;
          gate.reset();
          await Future<void>.delayed(Duration.zero);
          final sent = socket.messages
              .where((p) => p.first == 'EVENT')
              .toList();
          if (sent.isNotEmpty) {
            // Invalidation after the socket write must not erase accepted ACKs.
            valid = false;
            session.debugHandleMessage([
              'OK',
              (sent.single[1] as Map)['id'],
              true,
              '',
            ]);
          }
          await settled;
          expect(sent, hasLength(cancel ? 0 : 1));
          expect(accepted, cancel ? 0 : 1);
          if (kind == 9000) {
            // The membership object reports its own fence after the loop; the
            // irreversible write above still counts as accepted.
            expect(error, isA<StateError>());
          } else {
            // A message send that reached the socket is completed, not
            // cancelled, even though the operation expired after the enqueue.
            expect(error, cancel ? isA<StateError>() : isNull);
          }
        },
      );
    }
  }

  // The composer wraps ChannelActions in its own operation fence, so the
  // enqueue fence must consult the enclosing scope and the production
  // object's own callback. This is the only test-authored outer guard, and
  // it exists explicitly for that nesting contract.
  test('nested operation fences compose at actual enqueue', () async {
    for (final outerInvalid in [true, false]) {
      final gate = RelayRateLimitGate();
      final session = RelaySessionNotifier(rateLimitGate: gate);
      final socket = _Socket();
      session.debugAttachSocketForTest(socket);
      final relay = SignedEventRelay(
        session: session,
        nsec: nostr.Keys.generate().nsec,
      );
      var outerValid = true;
      var innerValid = true;
      var accepted = 0;
      final container = ProviderContainer();
      addTearDown(container.dispose);
      final actions = container.read(
        Provider(
          (ref) => ChannelActions(
            ref: ref,
            session: session,
            signedEventRelay: relay,
            currentPubkey: relay.pubkey,
            isCommunityValid: () => innerValid,
          ),
        ),
      );
      void ensureOuter() {
        if (!outerValid) throw StateError('outer operation expired');
      }

      gate.activate(10);
      final pending = withRelayPublicationGuard(
        ensureOuter,
        () => actions.addMembers(
          channelId: 'channel',
          pubkeys: ['target'],
          onAccepted: (_) => accepted++,
        ),
      );
      Object? error;
      final settled = pending.then<void>(
        (_) {},
        onError: (Object e) {
          error = e;
        },
      );
      expect(socket.messages, isEmpty);
      if (outerInvalid) {
        outerValid = false; // Enclosing operation expires during the wait.
      } else {
        innerValid = false; // Production object's own fence expires.
      }
      gate.reset();
      await Future<void>.delayed(Duration.zero);
      expect(socket.messages.where((p) => p.first == 'EVENT'), isEmpty);
      await settled;
      expect(accepted, 0);
      if (outerInvalid) {
        // The enclosing fence's failure is recorded per-pubkey and the
        // membership object surfaces it as an AddMembersException.
        final failure = error as AddMembersException;
        expect(failure.failures['target'], contains('outer operation expired'));
      } else {
        // The inner fence's failure aborts the whole add before the
        // per-pubkey failure report.
        expect(error, isA<StateError>());
      }
    }
  });
}

class _Socket extends RelaySocket {
  _Socket()
    : super(
        wsUrl: 'wss://relay.example',
        nsec: null,
        onMessage: (_) {},
        onConnected: () {},
        onDisconnected: (_) {},
      );
  final messages = <List<dynamic>>[];
  @override
  void send(List<dynamic> payload) => messages.add(payload);
}
