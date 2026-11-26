import 'dart:convert';
import 'dart:typed_data';

/// Canonical transcripts for Android push installations.
///
/// The gateway builds the same bytes with serde_json and verifies a signature
/// (or an attestation challenge) over them, so one differing byte is a silent
/// `401 invalid_attestation` or `404 not_authorized`. The encoding is
/// `<domain> + "\n" + <compact JSON object>` with members in a fixed order per
/// route. Ground truth is `app_attest_transcripts.json` in the gateway crate,
/// which the tests replay byte for byte.
///
/// `audience` is the origin and route registered by NIP-PL v1. A configured
/// gateway origin changes where requests go, never these bytes.
const _audienceOrigin = 'https://push.buzz.xyz';
const _wireVersion = 1;

const androidEnrollDomain = 'buzz.push.enroll.android.v1';
const androidDelegateDomain = 'buzz.push.delegate.v1';
const androidRotateEndpointDomain = 'buzz.push.rotate-endpoint.v1';
const androidRevokeDelegationDomain = 'buzz.push.revoke-delegation.v1';
const androidRevokeInstallationDomain = 'buzz.push.revoke-installation.v1';

/// The Android enrollment transcript. It has no key id: the Keystore key is
/// generated from this transcript's digest, and the gateway derives the id
/// from the attested key.
Uint8List androidEnrollTranscript({
  required String challengeId,
  required String challenge,
  required String endpoint,
  required int endpointEpoch,
  required int expiresAt,
}) => _transcript(androidEnrollDomain, [
  ('v', _wireVersion),
  ('audience', '$_audienceOrigin/v1/installations'),
  ('challenge_id', challengeId),
  ('challenge', challenge),
  ('endpoint', endpoint),
  ('endpoint_epoch', endpointEpoch),
  ('expires_at', expiresAt),
]);

Uint8List androidDelegateTranscript({
  required String challengeId,
  required String challenge,
  required String installationHandle,
  required int endpointEpoch,
  required int generation,
  required String relayPubkey,
  required int notBefore,
  required int expiresAt,
}) => _transcript(androidDelegateDomain, [
  ('v', _wireVersion),
  ('audience', '$_audienceOrigin/v1/delegations'),
  ('challenge_id', challengeId),
  ('challenge', challenge),
  ('installation_handle', installationHandle),
  ('endpoint_epoch', endpointEpoch),
  ('generation', generation),
  ('relay_pubkey', relayPubkey),
  ('not_before', notBefore),
  ('expires_at', expiresAt),
]);

Uint8List androidRotateEndpointTranscript({
  required String challengeId,
  required String challenge,
  required String installationHandle,
  required int endpointEpoch,
  required int newEndpointEpoch,
  required String endpoint,
}) => _transcript(androidRotateEndpointDomain, [
  ('v', _wireVersion),
  ('audience', '$_audienceOrigin/v1/installations/endpoint'),
  ('challenge_id', challengeId),
  ('challenge', challenge),
  ('installation_handle', installationHandle),
  ('endpoint_epoch', endpointEpoch),
  ('new_endpoint_epoch', newEndpointEpoch),
  ('endpoint', endpoint),
]);

Uint8List androidRevokeDelegationTranscript({
  required String challengeId,
  required String challenge,
  required String installationHandle,
  required String relayPubkey,
  required int generation,
}) => _transcript(androidRevokeDelegationDomain, [
  ('v', _wireVersion),
  ('audience', '$_audienceOrigin/v1/delegations/revoke'),
  ('challenge_id', challengeId),
  ('challenge', challenge),
  ('installation_handle', installationHandle),
  ('relay_pubkey', relayPubkey),
  ('generation', generation),
]);

Uint8List androidRevokeInstallationTranscript({
  required String challengeId,
  required String challenge,
  required String installationHandle,
  required int endpointEpoch,
  required int newEndpointEpoch,
}) => _transcript(androidRevokeInstallationDomain, [
  ('v', _wireVersion),
  ('audience', '$_audienceOrigin/v1/installations/revoke'),
  ('challenge_id', challengeId),
  ('challenge', challenge),
  ('installation_handle', installationHandle),
  ('endpoint_epoch', endpointEpoch),
  ('new_endpoint_epoch', newEndpointEpoch),
]);

Uint8List _transcript(String domain, List<(String, Object)> members) {
  final body = StringBuffer('{');
  for (var index = 0; index < members.length; index++) {
    final (name, value) = members[index];
    if (index > 0) body.write(',');
    body
      ..write('"$name":')
      ..write(switch (value) {
        final int number => '$number',
        final String text => _jsonString(name, text),
        _ => throw ArgumentError.value(value, name, 'Unsupported type'),
      });
  }
  body.write('}');
  return Uint8List.fromList(utf8.encode('$domain\n$body'));
}

/// Authority-bearing strings are printable ASCII. Anything else is a bug in
/// the caller, so it fails here rather than guessing at an escaping the
/// gateway might not reproduce.
String _jsonString(String field, String value) {
  final out = StringBuffer('"');
  for (final unit in value.codeUnits) {
    if (unit < 0x20 || unit > 0x7e) {
      throw FormatException('Transcript field $field is not printable ASCII.');
    }
    if (unit == 0x22 || unit == 0x5c) out.writeCharCode(0x5c);
    out.writeCharCode(unit);
  }
  return (out..write('"')).toString();
}
