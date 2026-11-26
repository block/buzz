import 'dart:convert';
import 'dart:io';

import 'package:buzz/shared/push/android_push_transcript.dart';
import 'package:crypto/crypto.dart' as crypto;
import 'package:flutter_test/flutter_test.dart';

/// The gateway's encoder and this one must agree to the byte: the gateway
/// verifies signatures and attestation challenges over its own copy.
void main() {
  final file = File(
    '../crates/buzz-push-gateway/tests/vectors/app_attest_transcripts.json',
  );
  final fixture = jsonDecode(file.readAsStringSync()) as Map<String, Object?>;
  final inputs = fixture['inputs']! as Map<String, Object?>;
  final challengeId = inputs['challenge_id']! as String;
  final challenge = inputs['challenge']! as String;
  final handle = inputs['installation_handle']! as String;
  final endpoint = inputs['endpoint']! as String;
  final relay = inputs['relay_pubkey']! as String;
  final notBefore = inputs['not_before']! as int;
  final expiresAt = inputs['expires_at']! as int;

  final encoders = <String, List<int> Function()>{
    'enroll_android': () => androidEnrollTranscript(
      challengeId: challengeId,
      challenge: challenge,
      endpoint: endpoint,
      endpointEpoch: 1,
      expiresAt: expiresAt,
    ),
    'delegate': () => androidDelegateTranscript(
      challengeId: challengeId,
      challenge: challenge,
      installationHandle: handle,
      endpointEpoch: 1,
      generation: 1,
      relayPubkey: relay,
      notBefore: notBefore,
      expiresAt: expiresAt,
    ),
    'rotate_endpoint': () => androidRotateEndpointTranscript(
      challengeId: challengeId,
      challenge: challenge,
      installationHandle: handle,
      endpointEpoch: 1,
      newEndpointEpoch: 2,
      endpoint: endpoint,
    ),
    'revoke_delegation': () => androidRevokeDelegationTranscript(
      challengeId: challengeId,
      challenge: challenge,
      installationHandle: handle,
      relayPubkey: relay,
      generation: 2,
    ),
    'revoke_installation': () => androidRevokeInstallationTranscript(
      challengeId: challengeId,
      challenge: challenge,
      installationHandle: handle,
      endpointEpoch: 1,
      newEndpointEpoch: 2,
    ),
  };

  for (final MapEntry(:key, :value) in encoders.entries) {
    test('$key transcript matches the gateway vector', () {
      final vector = (fixture['vectors']! as List)
          .cast<Map<String, Object?>>()
          .singleWhere((vector) => vector['name'] == key);
      final bytes = value();
      expect(utf8.decode(bytes), vector['transcript']);
      expect(crypto.sha256.convert(bytes).toString(), vector['sha256']);
    });
  }

  test('a field that is not printable ASCII fails instead of escaping', () {
    expect(
      () => androidEnrollTranscript(
        challengeId: challengeId,
        challenge: challenge,
        endpoint: 'tokené',
        endpointEpoch: 1,
        expiresAt: expiresAt,
      ),
      throwsFormatException,
    );
  });
}
