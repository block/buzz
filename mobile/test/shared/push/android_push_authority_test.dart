import 'dart:convert';

import 'package:buzz/shared/push/android_push_authority.dart';
import 'package:buzz/shared/push/android_push_gateway.dart';
import 'package:buzz/shared/push/android_push_keystore.dart';
import 'package:flutter_test/flutter_test.dart';

import '../../helpers/android_push_fakes.dart';

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
  }) => authority.enroll(
    relayOrigin: relay,
    relayPubkey: pubkey ?? (relay == _relayA ? _keyA : _keyB),
    endpoint: token,
  );

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
      gateway.failWith['/v1/installations'] = 503;
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
      gateway.failWith.remove('/v1/installations');

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
}
