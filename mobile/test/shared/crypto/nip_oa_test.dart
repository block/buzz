import 'dart:convert';
import 'dart:typed_data';

import 'package:flutter_test/flutter_test.dart';
import 'package:nostr/nostr.dart' as nostr;
import 'package:pointycastle/digests/sha256.dart';
import 'package:buzz/shared/crypto/nip_oa.dart';

String _sha256Hex(String input) {
  final digest = SHA256Digest().process(Uint8List.fromList(utf8.encode(input)));
  return digest.map((b) => b.toRadixString(16).padLeft(2, '0')).join();
}

List<String> authTag(
  nostr.Keys owner,
  String agentPubkey, {
  String conditions = '',
}) {
  final message = _sha256Hex('nostr:agent-auth:$agentPubkey:$conditions');
  final sig = nostr.Schnorr.sign(secretKey: owner.secret, message: message);
  return ['auth', owner.public, conditions, sig];
}

void main() {
  final owner = nostr.Keys.generate();
  final agent = nostr.Keys.generate();

  test('returns the owner pubkey for a valid auth tag', () {
    final tag = authTag(owner, agent.public);
    expect(
      verifiedOaOwnerPubkey([tag], agent.public),
      owner.public.toLowerCase(),
    );
  });

  test('accepts valid conditions strings', () {
    final tag = authTag(owner, agent.public, conditions: 'kind=0');
    expect(
      verifiedOaOwnerPubkey([tag], agent.public),
      owner.public.toLowerCase(),
    );
  });

  test('rejects a signature over a different agent pubkey', () {
    final otherAgent = nostr.Keys.generate();
    final tag = authTag(owner, otherAgent.public);
    expect(verifiedOaOwnerPubkey([tag], agent.public), isNull);
  });

  test('rejects a tampered signature', () {
    final tag = authTag(owner, agent.public);
    final tampered = [...tag];
    tampered[3] = tampered[3].replaceRange(
      0,
      1,
      tampered[3][0] == '0' ? '1' : '0',
    );
    expect(verifiedOaOwnerPubkey([tampered], agent.public), isNull);
  });

  test('rejects self-attestation', () {
    final tag = authTag(agent, agent.public);
    expect(verifiedOaOwnerPubkey([tag], agent.public), isNull);
  });

  test('rejects malformed conditions', () {
    final tag = authTag(owner, agent.public, conditions: 'kind=abc');
    expect(verifiedOaOwnerPubkey([tag], agent.public), isNull);
  });

  test('ignores unrelated tags', () {
    expect(
      verifiedOaOwnerPubkey([
        ['p', owner.public],
      ], agent.public),
      isNull,
    );
  });
}
