import 'dart:async';
import 'dart:convert';

import 'package:buzz/shared/push/android_push_authority.dart';
import 'package:buzz/shared/push/android_push_gateway.dart';
import 'package:buzz/shared/push/android_push_keystore.dart';
import 'package:flutter_test/flutter_test.dart';

import '../../helpers/android_push_fakes.dart';
import '../community/community_storage_test.dart' show FakeSecureStorage;

class _MemoryStore implements AndroidPushStateStore {
  String? value;

  @override
  Future<String?> read() async => value;

  @override
  Future<void> write(String? next) async => value = next;
}

const _relayA = 'wss://relay-a.example';
const _relayB = 'wss://relay-b.example';
final _keyA = 'a' * 64;
final _keyB = 'b' * 64;
final _keyC = 'c' * 64;

void main() {
  late int now;
  late FakePushGateway gateway;
  late FakeAndroidKeystore keystore;
  late _MemoryStore store;
  late List<Duration> sleeps;
  late AndroidPushAuthority authority;

  setUp(() {
    now = 1_750_000_000;
    gateway = FakePushGateway(nowSeconds: () => now);
    keystore = FakeAndroidKeystore();
    store = _MemoryStore();
    sleeps = [];
    authority = AndroidPushAuthority(
      keystore: keystore,
      gateway: fakeGatewayClient(gateway),
      store: store,
      now: () => DateTime.fromMillisecondsSinceEpoch(now * 1000),
      sleep: (delay) async => sleeps.add(delay),
    );
  });

  Future<AndroidPushGrant> enroll({
    String relay = _relayA,
    String? pubkey,
    String token = 'token-1',
  }) async => (await authority.enroll(
    relayOrigin: relay,
    relayPubkey: pubkey ?? (relay == _relayA ? _keyA : _keyB),
    endpoint: token,
    stillWanted: () => true,
    desiredRelays: () => {_relayA, _relayB},
  ))!;

  test('first enrollment, a second relay, reuse and renewal', () async {
    final first = await enroll();

    // The first enrollment waits a bounded random time, then enrolls with an
    // attestation over the transcript's digest and delegates once.
    expect(sleeps.single, lessThan(androidPushFirstEnrollmentJitterMax));
    expect(gateway.installations.values.single.token, 'token-1');
    expect(
      gateway.bodiesFor('/v1/installations').single,
      isNot(contains('key_id')),
    );
    expect(first.generation, 1);
    expect(first.endpointEpoch, 1);
    expect(first.relayOrigin, _relayA);
    expect(first.installationId, matches(RegExp(r'^[0-9a-f]{32}$')));
    expect(first.expiresAt, greaterThan(now + 86400));
    expect(first.endpointHash, hasLength(64));

    // The same relay again reuses the grant without calling the gateway; a
    // second relay delegates from the same installation.
    final calls = gateway.requests.length;
    expect((await enroll()).endpointGrant, first.endpointGrant);
    expect(gateway.requests.length, calls);
    final second = await enroll(relay: _relayB);
    expect(gateway.installations, hasLength(1));
    expect(second.generation, 1);
    expect(sleeps, hasLength(1));
    expect((await authority.grants()).map((g) => g.relayOrigin), {
      _relayA,
      _relayB,
    });

    // In the grant's final minutes the next call delegates again at a higher
    // generation.
    now = first.expiresAt - 200;
    final renewed = await enroll();
    expect(renewed.generation, 2);
    expect(renewed.endpointGrant, isNot(first.endpointGrant));

    // A renewal the gateway accepted, with its response lost, extended the
    // installation, so the phone keeps signing for it past the last expiry
    // it saw confirmed rather than enrolling again over the same token.
    now = renewed.expiresAt - 200;
    gateway.loseResponseFor.add('/v1/delegations');
    await expectLater(enroll(), throwsA(isA<Exception>()));
    now = renewed.expiresAt + 60;
    final resumed = await enroll();
    expect(resumed.generation, 4);
    expect(gateway.installations, hasLength(1));
    expect(keystore.generated, 1);
  });

  test('reading grants does not wait behind a queued enrollment', () async {
    final release = Completer<void>();
    authority = AndroidPushAuthority(
      keystore: keystore,
      gateway: fakeGatewayClient(gateway),
      store: store,
      now: () => DateTime.fromMillisecondsSinceEpoch(now * 1000),
      sleep: (_) => release.future,
    );
    final enrolling = enroll();
    await pumpEventQueue();

    expect(await authority.grants(), isEmpty);

    release.complete();
    await enrolling;
    expect(await authority.grants(), hasLength(1));
  });

  test('a lost enrollment response is resumed with the same request', () async {
    gateway.loseResponseFor.add('/v1/installations');
    await expectLater(enroll(), throwsA(isA<Exception>()));
    expect(store.value, contains('pendingEnrollment'));

    final grant = await enroll();

    final bodies = gateway.bodiesFor('/v1/installations');
    expect(bodies, hasLength(2));
    expect(jsonEncode(bodies[1]), jsonEncode(bodies[0]));
    expect(keystore.generated, 1);
    expect(gateway.installations, hasLength(1));
    expect(grant.generation, 1);
  });

  test(
    'an unavailable gateway keeps the journal and a refusal clears it',
    () async {
      // Even when the refusal's body is too large to read.
      for (final oversized in [false, true]) {
        gateway
          ..oversizeFailures = oversized
          ..failWith['/v1/installations'] = 503;
        await expectLater(
          enroll(),
          throwsA(
            isA<AndroidPushGatewayRejection>().having(
              (e) => e.isTransient,
              'transient',
              true,
            ),
          ),
        );
        expect(store.value, contains('pendingEnrollment'));
        expect(keystore.key, isNotNull);
      }

      gateway.failWith['/v1/installations'] = 403;
      await expectLater(enroll(), throwsA(isA<AndroidPushGatewayRejection>()));
      expect(keystore.key, isNull);
      expect(store.value, isNot(contains('pendingEnrollment')));
      gateway
        ..oversizeFailures = false
        ..failWith.remove('/v1/installations');

      gateway.refuseAttestation = true;
      await expectLater(
        enroll(),
        throwsA(isA<AndroidPushAttestationUnavailable>()),
      );
      expect(keystore.key, isNull);
      expect(store.value, isNot(contains('pendingEnrollment')));
      expect(gateway.installations, isEmpty);

      gateway.refuseAttestation = false;
      await enroll();
      expect(gateway.installations, hasLength(1));
    },
  );

  test(
    'a phone that cannot attest never contacts the enrollment route',
    () async {
      keystore.attests = false;
      await expectLater(
        enroll(),
        throwsA(isA<AndroidPushAttestationUnavailable>()),
      );
      expect(gateway.bodiesFor('/v1/installations'), isEmpty);
      expect(await authority.grants(), isEmpty);
    },
  );

  test('a new FCM token rotates the endpoint and re-delegates', () async {
    final first = await enroll();
    await enroll(relay: _relayB);

    final rotated = await enroll(token: 'token-2');

    final installation = gateway.installations.values.single;
    expect(installation.token, 'token-2');
    expect(installation.epoch, 2);
    expect(rotated.endpointEpoch, 2);
    expect(rotated.generation, 2);
    expect(rotated.endpointHash, isNot(first.endpointHash));
    // The other relay's grant was void at the old epoch, so it is replaced on
    // its next enrollment rather than reused.
    final other = await enroll(relay: _relayB, token: 'token-2');
    expect(other.endpointEpoch, 2);
    expect(other.generation, 2);
    expect(gateway.bodiesFor('/v1/installations'), hasLength(1));
  });

  test('a lost rotation response is accounted for on the retry', () async {
    await enroll();
    gateway.loseResponseFor.add('/v1/installations/endpoint');
    await expectLater(enroll(token: 'token-2'), throwsA(isA<Exception>()));
    expect(gateway.installations.values.single.token, 'token-2');

    // The retry's rotation is refused (the gateway is already at the next
    // epoch), the delegation at that epoch succeeds, and nothing re-enrolls.
    final grant = await enroll(token: 'token-2');

    expect(grant.endpointEpoch, 2);
    expect(gateway.bodiesFor('/v1/installations'), hasLength(1));
    expect(gateway.installations.values.single.token, 'token-2');
  });

  test(
    'a token that changed again before a lost rotation was settled',
    () async {
      await enroll();
      gateway.loseResponseFor.add('/v1/installations/endpoint');
      await expectLater(enroll(token: 'token-2'), throwsA(isA<Exception>()));

      final grant = await enroll(token: 'token-3');

      final installation = gateway.installations.values.single;
      expect(installation.token, 'token-3');
      expect(installation.epoch, 3);
      expect(grant.endpointEpoch, 3);
    },
  );

  test(
    'an installation the gateway dropped is replaced by a new one',
    () async {
      final first = await enroll();
      gateway.installations.values.single.revoked = true;
      // The phone only finds out when it next needs the gateway.
      now = first.expiresAt - 200;

      final grant = await enroll();

      expect(gateway.installations, hasLength(2));
      expect(keystore.generated, 2);
      expect(grant.generation, 1);
      expect(grant.endpointGrant, isNot(first.endpointGrant));
      expect(grant.installationId, first.installationId);
    },
  );

  test('a lost Keystore key is replaced, not signed around', () async {
    await enroll();
    keystore.key = null;
    // The gateway still holds the old installation for this token, so the
    // new one is refused loudly instead of the old authority being reused.
    await expectLater(
      enroll(),
      throwsA(
        isA<AndroidPushGatewayRejection>().having(
          (e) => e.status,
          'status',
          409,
        ),
      ),
    );
  });

  test(
    'sign-out revokes delegations, then the installation, and frees the token',
    () async {
      final a = await enroll();
      await enroll(relay: _relayB);

      // One community stays: only the other relay's delegation goes.
      await authority.reconcile(
        desiredRelays: () => {_relayA},
        revokeInstallation: () => false,
      );
      final installation = gateway.installations.values.single;
      expect(installation.revokedDelegations, {'$_keyB/1'});
      expect(installation.revoked, isFalse);

      // The community comes back, and its first delegation's response is
      // lost: the retry delegates again instead of reusing the revoked grant.
      gateway.loseResponseFor.add('/v1/delegations');
      await expectLater(enroll(relay: _relayB), throwsA(isA<Exception>()));
      final b = await enroll(relay: _relayB);
      expect(b.generation, 3);
      expect(installation.delegations[_keyB], 3);

      // The last community goes: the installation is revoked, the key deleted.
      gateway.loseResponseFor.add('/v1/installations/revoke');
      await expectLater(
        authority.reconcile(
          desiredRelays: () => {},
          revokeInstallation: () => true,
        ),
        throwsA(isA<Exception>()),
      );
      await authority.reconcile(
        desiredRelays: () => {},
        revokeInstallation: () => true,
      );
      expect(installation.revoked, isTrue);
      expect(keystore.key, isNull);

      // Lease tombstones still read the relay's installation id afterwards, and
      // a later sign-in enrolls the same token again.
      expect(
        (await authority.grants()).map((g) => g.installationId),
        contains(a.installationId),
      );
      final again = await enroll();
      expect(gateway.installations, hasLength(2));
      expect(again.generation, 1);
    },
  );

  test('sign-out settles a rotation whose response was lost', () async {
    await enroll();
    gateway.loseResponseFor.add('/v1/installations/endpoint');
    await expectLater(enroll(token: 'token-2'), throwsA(isA<Exception>()));
    final installation = gateway.installations.values.single;
    expect(installation.epoch, 2);

    // The stored epoch is stale. The installation is still revoked, and the
    // token it held is free again.
    await authority.reconcile(
      desiredRelays: () => {},
      revokeInstallation: () => true,
    );

    expect(installation.revoked, isTrue);
    expect(keystore.key, isNull);
    await enroll(token: 'token-2');
    expect(gateway.installations, hasLength(2));
  });

  test('sign-out revokes an installation whose enrollment was lost', () async {
    gateway.loseResponseFor.add('/v1/installations');
    await expectLater(enroll(), throwsA(isA<Exception>()));
    final installation = gateway.installations.values.single;

    await authority.reconcile(
      desiredRelays: () => {},
      revokeInstallation: () => true,
    );

    expect(installation.revoked, isTrue);
    expect(keystore.key, isNull);
    expect(gateway.bodiesFor('/v1/installations'), hasLength(2));
    await enroll();
    expect(gateway.installations, hasLength(2));
  });

  test(
    'a revocation whose delegation response was lost still finds it',
    () async {
      await enroll();
      gateway.loseResponseFor.add('/v1/delegations');
      now +=
          400 * 24 * 3600; // far past the grant, so enrolling delegates again
      // The installation expired too, so this re-enrolls, then loses the
      // delegation response after the gateway accepted generation 1.
      await expectLater(enroll(), throwsA(isA<Exception>()));

      await authority.reconcile(
        desiredRelays: () => {},
        revokeInstallation: () => false,
      );

      expect(gateway.installations.values.last.revokedDelegations, {
        '$_keyA/1',
      });
    },
  );

  // ---- delegations are keyed by executor key ----

  test('communities on one relay share the delegation of its key', () async {
    await enroll(relay: _relayA, pubkey: _keyA);
    await enroll(relay: _relayB, pubkey: _keyA);

    // The gateway keys a delegation by relay key, so one call serves both.
    expect(gateway.bodiesFor('/v1/delegations'), hasLength(1));
    expect(gateway.installations, hasLength(1));
    final installation = gateway.installations.values.single;

    Future<void> reconcile(Set<String> desired) => authority.reconcile(
      desiredRelays: () => desired,
      revokeInstallation: () => false,
    );
    await reconcile({_relayB});
    await reconcile({_relayA});
    expect(installation.revokedDelegations, isEmpty);

    await reconcile({});
    expect(installation.revokedDelegations, {'$_keyA/1'});
    // Tombstones still read each origin's grant afterwards.
    expect((await authority.grants()).map((g) => g.relayOrigin), {
      _relayA,
      _relayB,
    });
  });

  test('a rotated relay key is revoked, and so is its successor', () async {
    await enroll(relay: _relayA, pubkey: _keyA);
    await enroll(relay: _relayB, pubkey: _keyC);
    final rotated = await enroll(relay: _relayA, pubkey: _keyB);
    expect(rotated.relayPubkey, _keyB);
    final installation = gateway.installations.values.single;

    Future<void> reconcile(Set<String> desired) => authority.reconcile(
      desiredRelays: () => desired,
      revokeInstallation: () => false,
    );
    await reconcile({_relayA, _relayB});
    expect(installation.revokedDelegations, {'$_keyA/1'});

    // The community on the other key stays, so only the first one's goes.
    await reconcile({_relayB});
    expect(installation.revokedDelegations, {'$_keyA/1', '$_keyB/1'});
  });

  test('a lost delegation response for a new key is revoked too', () async {
    await enroll(relay: _relayA, pubkey: _keyA);
    gateway.loseResponseFor.add('/v1/delegations');
    await expectLater(
      enroll(relay: _relayA, pubkey: _keyB),
      throwsA(isA<Exception>()),
    );

    await authority.reconcile(
      desiredRelays: () => {},
      revokeInstallation: () => false,
    );

    // The gateway accepted the key's first generation, which was never stored
    // as accepted, so it is the spent one that is tried.
    expect(gateway.installations.values.single.revokedDelegations, {
      '$_keyA/1',
      '$_keyB/1',
    });
  });

  test('state from the previous storage key is dropped', () async {
    final secure = FakeSecureStorage();
    await secure.write(key: 'buzz_android_push_authority_v1', value: '{}');
    final store = SecureAndroidPushStateStore(secure: secure);

    expect(await store.read(), isNull);

    expect(await secure.read(key: 'buzz_android_push_authority_v1'), isNull);
    await store.write('state');
    expect(await store.read(), 'state');
    expect(await secure.read(key: 'buzz_android_push_authority_v2'), 'state');
  });

  // ---- opt-in fence ----

  test(
    'an opt-out during the enrollment jitter leaves nothing behind',
    () async {
      final release = Completer<void>();
      authority = AndroidPushAuthority(
        keystore: keystore,
        gateway: fakeGatewayClient(gateway),
        store: store,
        now: () => DateTime.fromMillisecondsSinceEpoch(now * 1000),
        sleep: (_) => release.future,
      );
      var wanted = true;
      final enrolling = authority.enroll(
        relayOrigin: _relayA,
        relayPubkey: _keyA,
        endpoint: 'token-1',
        stillWanted: () => wanted,
        desiredRelays: () => {_relayA},
      );
      await pumpEventQueue();
      wanted = false;
      release.complete();

      expect(await enrolling, isNull);
      expect(gateway.requests, isEmpty);
      expect(keystore.generated, 0);
      expect(await authority.grants(), isEmpty);
    },
  );

  test(
    'the enrolling relay keeps its delegation before the caller lists it',
    () async {
      // A community's relay may not be in the desired list yet when its first
      // enrollment finishes; while it is still wanted, its new delegation is
      // not swept.
      final grant = (await authority.enroll(
        relayOrigin: _relayA,
        relayPubkey: _keyA,
        endpoint: 'token-1',
        stillWanted: () => true,
        desiredRelays: () => {},
      ))!;
      final installation = gateway.installations.values.single;
      expect(installation.revokedDelegations, isEmpty);
      expect(grant.generation, 1);

      // An opt-out that lands while the next delegation is in flight sees
      // that delegation revoked by the same sweep.
      await authority.enroll(
        relayOrigin: _relayB,
        relayPubkey: _keyB,
        endpoint: 'token-1',
        stillWanted: () => gateway.bodiesFor('/v1/delegations').length < 2,
        desiredRelays: () => {_relayA},
      );
      expect(installation.revokedDelegations, {'$_keyB/1'});
    },
  );

  test('an opt-out after enrollment creates no delegation', () async {
    final enrolling = authority.enroll(
      relayOrigin: _relayA,
      relayPubkey: _keyA,
      endpoint: 'token-1',
      // Wanted until the installation exists.
      stillWanted: () => gateway.installations.isEmpty,
      desiredRelays: () => {_relayA},
    );

    expect(await enrolling, isNull);
    expect(gateway.installations, hasLength(1));
    expect(gateway.bodiesFor('/v1/delegations'), isEmpty);
    expect(await authority.grants(), isEmpty);
  });

  // ---- gateway binding ----

  test('state from another gateway is dropped, not reused', () async {
    final first = await enroll();
    final other = FakePushGateway(nowSeconds: () => now);
    authority = AndroidPushAuthority(
      keystore: keystore,
      gateway: fakeGatewayClient(other, baseUrl: 'https://push-b.example'),
      store: store,
      now: () => DateTime.fromMillisecondsSinceEpoch(now * 1000),
      sleep: (_) async {},
    );

    // Nothing of the first gateway's is readable, and nothing is revoked on
    // either gateway: its installation just expires there.
    expect(await authority.grants(), isEmpty);
    final grant = await enroll();

    expect(keystore.deleted, 1);
    expect(keystore.generated, 2);
    expect(other.installations, hasLength(1));
    expect(other.bodiesFor('/v1/delegations').single['generation'], 1);
    expect(grant.installationId, isNot(first.installationId));
    expect(gateway.installations.values.single.revoked, isFalse);
    expect(store.value, contains('https://push-b.example'));
  });

  // ---- a refused delegation ----

  test('a delegation refused once is retried with a fresh challenge', () async {
    final first = await enroll();
    now = first.expiresAt - 200;
    gateway.refuseOnce.add('/v1/delegations');

    final renewed = await enroll();

    // A challenge that expired while the app stalled is not a lost
    // installation: the key stays and the same installation is used.
    final bodies = gateway.bodiesFor('/v1/delegations');
    expect(bodies, hasLength(3));
    expect(bodies[2]['challenge_id'], isNot(bodies[1]['challenge_id']));
    expect(renewed.generation, 3);
    expect(keystore.deleted, 0);
    expect(gateway.installations, hasLength(1));
  });
}
