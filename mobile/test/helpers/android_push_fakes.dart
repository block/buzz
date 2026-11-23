import 'dart:convert';
import 'dart:math';
import 'dart:typed_data';

import 'package:buzz/shared/push/android_push_gateway.dart';
import 'package:buzz/shared/push/android_push_keystore.dart';
import 'package:buzz/shared/push/android_push_transcript.dart';
import 'package:crypto/crypto.dart' as crypto;
import 'package:http/http.dart' as http;
import 'package:http/testing.dart';

/// A Keystore stand-in. Its "signature" is the digest of the public key and
/// the bytes signed, which [FakePushGateway] recomputes from the transcript it
/// rebuilds, so a call signed over the wrong bytes is refused. Its chain is the
/// public key and the attestation challenge, like the gateway test verifier's.
class FakeAndroidKeystore implements AndroidPushKeystore {
  AndroidPushKey? key;
  int generated = 0;
  int deleted = 0;
  bool attests = true;

  @override
  Future<AndroidPushKey?> current() async => key;

  @override
  Future<AndroidPushKey> generate(Uint8List attestationChallenge) async {
    if (!attests) {
      throw const AndroidPushAttestationUnavailable('no attestation chain');
    }
    generated++;
    final public = Uint8List.fromList([
      4,
      ...crypto.sha256.convert(utf8.encode('key-$generated')).bytes,
      ...crypto.sha256.convert(utf8.encode('key-y-$generated')).bytes,
    ]);
    return key = AndroidPushKey(
      publicKey: public,
      chain: [public, attestationChallenge],
    );
  }

  @override
  Future<String> sign(Uint8List data) async {
    final current = key;
    if (current == null) throw StateError('no key');
    return signatureFor(current.publicKey, data);
  }

  @override
  Future<void> delete() async {
    deleted++;
    key = null;
  }
}

String signatureFor(Uint8List publicKey, List<int> transcript) =>
    base64.encode(crypto.sha256.convert([...publicKey, ...transcript]).bytes);

class _Installation {
  _Installation(this.handle, this.publicKey, this.token, this.expiresAt);

  final String handle;
  final Uint8List publicKey;
  String token;
  int expiresAt;
  int epoch = 1;
  bool revoked = false;
  final delegations = <String, int>{};
  final revokedDelegations = <String>{};
}

/// The gateway's installation semantics, behind an HTTP client: one active
/// installation per token and per key, enrollment replay, single-use
/// challenges, strictly increasing delegation generations at the installation's
/// epoch, rotation by exactly one epoch, idempotent revocation, and a
/// not_authorized 404 for every refused signed call.
class FakePushGateway {
  FakePushGateway({required this.nowSeconds});

  final int Function() nowSeconds;
  final installations = <String, _Installation>{};
  final requests = <({String route, Map<String, Object?> body})>[];
  final _challenges = <String, String>{};
  var _next = 0;

  /// Routes whose next response is lost after the gateway acted on it.
  final loseResponseFor = <String>{};

  /// Routes that answer with this status instead of acting.
  final failWith = <String, int>{};

  /// Pad [failWith] answers past the client's 64 KiB read limit.
  bool oversizeFailures = false;

  /// Routes whose next call is refused like an expired challenge: answered
  /// `not_authorized` without acting.
  final refuseOnce = <String>{};

  /// Routes whose next response waits for the future after the gateway acted.
  final holdResponseFor = <String, Future<void>>{};

  /// Refuse attestations like a gateway that does not trust this phone.
  bool refuseAttestation = false;

  List<Map<String, Object?>> bodiesFor(String route) => [
    for (final request in requests)
      if (request.route == route) request.body,
  ];

  late final http.Client client = MockClient((request) async {
    final route = request.url.path;
    final body = jsonDecode(request.body) as Map<String, Object?>;
    requests.add((route: route, body: body));
    final status = failWith[route];
    if (status != null) {
      return _json(status, {
        'error': 'unavailable',
        if (oversizeFailures) 'padding': 'x' * 65537,
      });
    }
    if (refuseOnce.remove(route)) return _notAuthorized();
    final response = _handle(route, body);
    await holdResponseFor.remove(route);
    if (loseResponseFor.remove(route)) {
      throw http.ClientException('connection lost');
    }
    return response;
  });

  http.Response _json(int status, Map<String, Object?> body) =>
      http.Response(jsonEncode(body), status);

  http.Response _notAuthorized() => _json(404, {'error': 'not_authorized'});

  http.Response _handle(String route, Map<String, Object?> body) {
    if (route == '/v1/installations/challenges') {
      final id = 'challenge-${_next++}';
      final text = 'text-$id';
      _challenges[id] = text;
      return _json(200, {
        'challenge_id': id,
        'challenge': text,
        'expires_at': nowSeconds() + 300,
      });
    }
    if (route == '/v1/installations') return _enroll(body);
    final installation = installations[body['installation_handle']];
    final live =
        installation != null &&
        installation.expiresAt >= nowSeconds() &&
        (!installation.revoked || route == '/v1/installations/revoke');
    if (!live || !_signatureOk(route, body, installation)) {
      return _notAuthorized();
    }
    switch (route) {
      case '/v1/delegations':
        final relay = body['relay_pubkey']! as String;
        final generation = body['generation']! as int;
        if (body['endpoint_epoch'] != installation.epoch ||
            generation <= (installation.delegations[relay] ?? 0)) {
          return _notAuthorized();
        }
        installation.delegations[relay] = generation;
        installation.revokedDelegations.remove('$relay/$generation');
        installation.expiresAt = max(
          installation.expiresAt,
          body['expires_at']! as int,
        );
        return _json(201, {
          'endpoint_grant':
              'grant:${installation.handle}:$relay:$generation:${installation.epoch}',
        });
      case '/v1/installations/endpoint':
        if (body['endpoint_epoch'] != installation.epoch ||
            body['new_endpoint_epoch'] != installation.epoch + 1) {
          return _notAuthorized();
        }
        installation.token = body['endpoint']! as String;
        installation.epoch++;
        return _json(200, {'status': 'rotated'});
      case '/v1/delegations/revoke':
        final relay = body['relay_pubkey']! as String;
        final generation = body['generation']! as int;
        if (installation.delegations[relay] != generation) {
          return _notAuthorized();
        }
        installation.revokedDelegations.add('$relay/$generation');
        return _json(200, {'status': 'revoked'});
      default:
        if (body['endpoint_epoch'] == installation.epoch &&
            body['new_endpoint_epoch'] == installation.epoch + 1) {
          installation.revoked = true;
          installation.epoch++;
        } else if (!(installation.revoked &&
            body['new_endpoint_epoch'] == installation.epoch)) {
          return _notAuthorized();
        }
        return _json(200, {'status': 'revoked'});
    }
  }

  http.Response _enroll(Map<String, Object?> body) {
    final chain = (body['android_attestation']! as List).cast<String>();
    final transcript = androidEnrollTranscript(
      challengeId: body['challenge_id']! as String,
      challenge: body['challenge']! as String,
      endpoint: body['endpoint']! as String,
      endpointEpoch: body['endpoint_epoch']! as int,
      expiresAt: body['expires_at']! as int,
    );
    final attested = base64.decode(chain[1]);
    if (refuseAttestation ||
        !_same(attested, crypto.sha256.convert(transcript).bytes)) {
      return _json(401, {'error': 'invalid_attestation'});
    }
    final publicKey = Uint8List.fromList(base64.decode(chain[0]));
    final token = body['endpoint']! as String;
    for (final other in installations.values) {
      if (other.revoked || other.expiresAt < nowSeconds()) continue;
      if (_same(other.publicKey, publicKey) && other.token == token) {
        return _json(201, {
          'installation_handle': other.handle,
          'endpoint_epoch': other.epoch,
          'expires_at': other.expiresAt,
        });
      }
      if (_same(other.publicKey, publicKey) || other.token == token) {
        return _json(409, {'error': 'conflict'});
      }
    }
    final handle = 'installation-${installations.length + 1}';
    installations[handle] = _Installation(
      handle,
      publicKey,
      token,
      body['expires_at']! as int,
    );
    return _json(201, {
      'installation_handle': handle,
      'endpoint_epoch': 1,
      'expires_at': body['expires_at'],
    });
  }

  bool _signatureOk(
    String route,
    Map<String, Object?> b,
    _Installation installation,
  ) {
    final common = (b['challenge_id']! as String, b['challenge']! as String);
    final handle = installation.handle;
    final transcript = switch (route) {
      '/v1/delegations' => androidDelegateTranscript(
        challengeId: common.$1,
        challenge: common.$2,
        installationHandle: handle,
        endpointEpoch: b['endpoint_epoch']! as int,
        generation: b['generation']! as int,
        relayPubkey: b['relay_pubkey']! as String,
        notBefore: b['not_before']! as int,
        expiresAt: b['expires_at']! as int,
      ),
      '/v1/installations/endpoint' => androidRotateEndpointTranscript(
        challengeId: common.$1,
        challenge: common.$2,
        installationHandle: handle,
        endpointEpoch: b['endpoint_epoch']! as int,
        newEndpointEpoch: b['new_endpoint_epoch']! as int,
        endpoint: b['endpoint']! as String,
      ),
      '/v1/delegations/revoke' => androidRevokeDelegationTranscript(
        challengeId: common.$1,
        challenge: common.$2,
        installationHandle: handle,
        relayPubkey: b['relay_pubkey']! as String,
        generation: b['generation']! as int,
      ),
      _ => androidRevokeInstallationTranscript(
        challengeId: common.$1,
        challenge: common.$2,
        installationHandle: handle,
        endpointEpoch: b['endpoint_epoch']! as int,
        newEndpointEpoch: b['new_endpoint_epoch']! as int,
      ),
    };
    // Challenges are single use, whether or not the call is accepted.
    final issued = _challenges.remove(common.$1);
    return issued == common.$2 &&
        b['assertion'] == signatureFor(installation.publicKey, transcript);
  }

  bool _same(List<int> a, List<int> b) =>
      a.length == b.length &&
      Iterable.generate(a.length).every((i) => a[i] == b[i]);
}

AndroidPushGatewayClient fakeGatewayClient(
  FakePushGateway gateway, {
  String baseUrl = 'https://push.example',
}) => AndroidPushGatewayClient(
  baseUrl: Uri.parse(baseUrl),
  client: gateway.client,
);
