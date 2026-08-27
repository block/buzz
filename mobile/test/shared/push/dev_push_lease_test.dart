import 'dart:convert';

import 'package:buzz/shared/auth/auth_provider.dart';
import 'package:buzz/shared/crypto/nip44.dart';
import 'package:buzz/shared/push/dev_push_lease.dart';
import 'package:buzz/shared/push/push_bridge.dart';
import 'package:buzz/shared/push/push_subscription.dart';
import 'package:buzz/shared/relay/nostr_models.dart';
import 'package:buzz/shared/relay/relay_session.dart';
import 'package:buzz/shared/relay/relay_socket.dart';
import 'package:buzz/shared/relay/signed_event_relay.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';
import 'package:nostr/nostr.dart' as nostr;

void main() {
  final signer = nostr.Keys.generate();
  final relay = nostr.Keys.generate();
  final descriptor = _descriptor(relay.public);
  final grant = _grant(relay.public);
  final now = DateTime.fromMillisecondsSinceEpoch(1752620000 * 1000);

  test('publishes strict kind-30350 lease and waits for accepted OK', () async {
    Map<String, dynamic>? submitted;
    final publication = await publishBuzzDevPushLease(
      grant: grant,
      leaseGeneration: 7,
      descriptor: descriptor,
      nsec: signer.nsec,
      memberPubkey: signer.public,
      subscriptions: [
        BuzzPushSubscription(
          filter: BuzzPushFilter(kinds: const [9], pTags: [signer.public]),
          notificationClass: 'default',
        ),
      ],
      now: () => now,
      submit:
          ({required kind, required content, required tags, createdAt}) async {
            submitted = {
              'kind': kind,
              'content': content,
              'tags': tags,
              'createdAt': createdAt,
            };
            return const NostrEvent(
              id: 'accepted-id',
              pubkey: '',
              createdAt: 0,
              kind: 0,
              tags: [],
              content: 'saved',
              sig: '',
            );
          },
    );

    expect(publication.eventId, 'accepted-id');
    expect(grant.relayOrigin, descriptor.origin);
    expect(jsonDecode(publication.plaintext)['origin'], descriptor.origin);
    expect(submitted!['kind'], buzzPushLeaseKind);
    expect(submitted!['createdAt'], 1752620000);
    expect(submitted!['tags'], [
      ['d', 'c' * 32],
      ['expiration', '1755212000'],
      ['exec', 'relay-v1'],
    ]);
    final plaintext = nip44Decrypt(
      getConversationKey(relay.secret, signer.public),
      submitted!['content'] as String,
    );
    expect(jsonDecode(plaintext), {
      'v': 1,
      'origin': 'wss://tenant.example:8443',
      'app_profile': 'buzz-ios-dogfood',
      'transport': 'apns',
      'endpoint': 'opaque-grant',
      'generation': 7,
      'active': true,
      'subscriptions': [
        {
          'filter': {
            'kinds': [9],
            '#p': [signer.public],
          },
          'class': 'default',
        },
      ],
    });
  });

  test('uses the community lease address instead of the endpoint id', () async {
    List<List<String>>? submittedTags;

    await publishBuzzDevPushLease(
      grant: grant,
      leaseInstallationId: 'e' * 32,
      descriptor: descriptor,
      nsec: signer.nsec,
      memberPubkey: signer.public,
      subscriptions: const [],
      now: () => now,
      submit:
          ({required kind, required content, required tags, createdAt}) async {
            submittedTags = tags;
            return const NostrEvent(
              id: 'accepted-id',
              pubkey: '',
              createdAt: 0,
              kind: 0,
              tags: [],
              content: 'saved',
              sig: '',
            );
          },
    );

    expect(submittedTags!.first, ['d', 'e' * 32]);
  });

  test('rejects a malformed community lease address', () async {
    await expectLater(
      publishBuzzDevPushLease(
        grant: grant,
        leaseInstallationId: 'malformed',
        descriptor: descriptor,
        nsec: signer.nsec,
        memberPubkey: signer.public,
        subscriptions: const [],
        now: () => now,
        submit:
            ({
              required kind,
              required content,
              required tags,
              createdAt,
            }) async => throw StateError('should not publish'),
      ),
      throwsFormatException,
    );
  });

  test(
    'mutation control rejects relay OK false then accepts restored event',
    () async {
      final events = <NostrEvent>[];
      final acknowledgements = <List<dynamic>>[];
      final session = RelaySessionNotifier();
      final container = ProviderContainer(
        overrides: [
          relaySessionProvider.overrideWith(() => session),
          authProvider.overrideWith(() => _UnauthenticatedAuthNotifier()),
        ],
      );
      await container.read(authProvider.future);
      final subscription = container.listen(relaySessionProvider, (_, _) {});
      addTearDown(subscription.close);
      addTearDown(container.dispose);
      final socket = _MutatingRelaySocket(
        events,
        onEvent: (event) => event.kind == 40002
            ? (accepted: false, message: 'invalid: kind not push-eligible')
            : (accepted: true, message: 'saved'),
        onAcknowledgement: acknowledgements.add,
      );
      session.debugAttachSocketForTest(socket);
      final relayClient = SignedEventRelay(session: session, nsec: signer.nsec);

      final mutated = relayClient.submit(
        kind: 40002,
        content: 'mutated',
        tags: const [],
        createdAt: 1752620000,
      );
      final rejection = expectLater(
        mutated,
        throwsA(
          isA<Exception>().having(
            (error) => error.toString(),
            'message',
            contains('invalid: kind not push-eligible'),
          ),
        ),
      );
      await _deliverAcknowledgement(acknowledgements, session);
      await rejection;
      final acceptedFuture = relayClient.submit(
        kind: buzzPushLeaseKind,
        content: 'restored',
        tags: [
          ['d', 'c' * 32],
          ['expiration', '1755212000'],
          ['exec', 'relay-v1'],
        ],
        createdAt: 1752620001,
      );
      await _deliverAcknowledgement(acknowledgements, session);
      final accepted = await acceptedFuture;

      expect(accepted.content, 'saved');
      expect(events.map((event) => event.kind), [40002, buzzPushLeaseKind]);
      expect(events.every((event) => event.pubkey == signer.public), isTrue);
      for (final event in events) {
        expect(
          () => nostr.Event(
            event.id,
            event.pubkey,
            event.createdAt,
            event.kind,
            event.tags,
            event.content,
            event.sig,
          ),
          returnsNormally,
        );
      }
    },
  );

  test('propagates relay rejection instead of accepting locally', () async {
    await expectLater(
      publishBuzzDevPushLease(
        grant: grant,
        descriptor: descriptor,
        nsec: signer.nsec,
        memberPubkey: signer.public,
        subscriptions: [
          BuzzPushSubscription(
            filter: BuzzPushFilter(kinds: const [9], pTags: [signer.public]),
            notificationClass: 'default',
          ),
        ],
        now: () => now,
        submit:
            ({
              required kind,
              required content,
              required tags,
              createdAt,
            }) async => throw Exception('invalid: origin mismatch'),
      ),
      throwsA(
        isA<Exception>().having(
          (error) => error.toString(),
          'message',
          contains('invalid: origin mismatch'),
        ),
      ),
    );
  });

  test('publishes a minimal higher-generation inactive tombstone', () async {
    Map<String, dynamic>? submitted;
    final publication = await publishBuzzPushLeaseTombstone(
      descriptor: descriptor,
      installationId: grant.installationId,
      generation: 3,
      nsec: signer.nsec,
      memberPubkey: signer.public,
      now: () => now,
      submit:
          ({required kind, required content, required tags, createdAt}) async {
            submitted = {
              'kind': kind,
              'content': content,
              'tags': tags,
              'createdAt': createdAt,
            };
            return const NostrEvent(
              id: 'tombstone-id',
              pubkey: '',
              createdAt: 0,
              kind: 0,
              tags: [],
              content: 'saved',
              sig: '',
            );
          },
    );

    expect(publication.eventId, 'tombstone-id');
    expect(jsonDecode(publication.plaintext), {
      'v': 1,
      'origin': descriptor.origin,
      'generation': 3,
      'active': false,
    });
    expect(submitted!['kind'], buzzPushLeaseKind);
    expect(submitted!['tags'], [
      ['d', grant.installationId],
      ['expiration', '1755212000'],
      ['exec', descriptor.executorKeyId],
    ]);
    final plaintext = nip44Decrypt(
      getConversationKey(relay.secret, signer.public),
      submitted!['content'] as String,
    );
    expect(jsonDecode(plaintext), jsonDecode(publication.plaintext));
  });

  test('descriptor rejects canonical origin with a trailing slash', () {
    final information = _descriptorJson(relay.public);
    (information['push'] as Map<String, dynamic>)['origin'] =
        'wss://tenant.example:8443/';

    expect(
      () => BuzzPushLeaseDescriptor.fromRelayInformation(information),
      throwsA(isA<FormatException>()),
    );
  });

  test('descriptor rejects unsupported h grammar', () {
    final information = _descriptorJson(relay.public);
    (information['push'] as Map<String, dynamic>)['h_grammar'] = 'opaque';

    expect(
      () => BuzzPushLeaseDescriptor.fromRelayInformation(information),
      throwsA(isA<FormatException>()),
    );
  });

  test('descriptor rejects unknown push fields', () {
    final information = _descriptorJson(relay.public);
    (information['push'] as Map<String, dynamic>)['future'] = true;

    expect(
      () => BuzzPushLeaseDescriptor.fromRelayInformation(information),
      throwsA(isA<FormatException>()),
    );
  });
}

class _UnauthenticatedAuthNotifier extends AuthNotifier {
  @override
  Future<AuthState> build() async =>
      const AuthState(status: AuthStatus.unauthenticated);
}

class _MutatingRelaySocket extends RelaySocket {
  final List<NostrEvent> events;
  final ({bool accepted, String message}) Function(NostrEvent event) onEvent;
  final void Function(List<dynamic> message) _onAcknowledgement;

  _MutatingRelaySocket(
    this.events, {
    required this.onEvent,
    required void Function(List<dynamic> message) onAcknowledgement,
  }) : _onAcknowledgement = onAcknowledgement,
       super(
         wsUrl: 'wss://tenant.example:8443',
         nsec: null,
         onMessage: _ignoreMessage,
         onConnected: _ignoreConnected,
         onDisconnected: _ignoreDisconnected,
       );

  @override
  SocketState get state => SocketState.connected;

  @override
  void send(List<dynamic> payload) {
    if (payload case ['EVENT', final Map<String, dynamic> eventJson]) {
      final event = NostrEvent.fromJson(eventJson);
      events.add(event);
      final response = onEvent(event);
      _onAcknowledgement(['OK', event.id, response.accepted, response.message]);
    }
  }

  @override
  Future<void> disconnect() async {}

  @override
  void dispose() {}
}

void _ignoreConnected() {}
void _ignoreDisconnected(Object? _) {}
void _ignoreMessage(List<dynamic> _) {}

Future<void> _deliverAcknowledgement(
  List<List<dynamic>> acknowledgements,
  RelaySessionNotifier session,
) async {
  while (acknowledgements.isEmpty) {
    await Future<void>.delayed(Duration.zero);
  }
  session.debugHandleMessage(acknowledgements.removeAt(0));
}

BuzzPushLeaseDescriptor _descriptor(String relayPubkey) =>
    BuzzPushLeaseDescriptor.fromRelayInformation(_descriptorJson(relayPubkey));

Map<String, dynamic> _descriptorJson(String relayPubkey) => {
  'supported_extensions': ['nip-er', 'nip-pl'],
  'push': {
    'origin': 'wss://tenant.example:8443',
    'keys': [
      {'id': 'relay-v1', 'pubkey': relayPubkey, 'current': true},
    ],
    'app_profiles': [
      {'id': 'buzz-ios-dogfood', 'transport': 'apns'},
    ],
    'push_kinds': [9, 40002, 45001, 45003],
    'h_grammar': 'uuid-v4-lowercase',
    'class_support': {
      'apns': ['default'],
    },
    'limitation': {
      'max_lease_ttl': 2592000,
      'max_leases_per_pubkey': 16,
      'max_subscriptions_per_lease': 16,
      'max_kinds': 16,
      'max_authors': 20,
      'max_h': 50,
      'max_tag_values': 20,
      'max_ignore': 8,
      'max_content_len': 65536,
      'max_plaintext_len': 32768,
      'max_endpoint_len': 4096,
      'max_string_len': 512,
    },
  },
};

BuzzPushEndpointGrant _grant(String relayPubkey) => BuzzPushEndpointGrant(
  relayOrigin: 'wss://tenant.example:8443',
  relayPubkey: relayPubkey,
  installationId: 'c' * 32,
  endpointGrant: 'opaque-grant',
  endpointHash: 'd' * 64,
  appProfile: 'buzz-ios-dogfood',
  endpointEpoch: 1,
  generation: 1,
  expiresAt: 1756212000,
);
