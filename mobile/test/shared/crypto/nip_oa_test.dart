import 'dart:convert';
import 'dart:typed_data';

import 'package:flutter_test/flutter_test.dart';
import 'package:nostr/nostr.dart' as nostr;
import 'package:pointycastle/digests/sha256.dart';
import 'package:buzz/shared/crypto/nip_oa.dart';
import 'package:buzz/shared/relay/relay.dart';

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

NostrEvent profile(
  nostr.Keys agent,
  List<List<String>> tags, {
  int createdAt = 100,
  int kind = 0,
}) => NostrEvent.fromJson(
  nostr.Event.from(
    kind: kind,
    content: '{}',
    secretKey: agent.secret,
    createdAt: createdAt,
    tags: tags,
  ).toMap(),
);

void main() {
  final owner = nostr.Keys.generate();
  final agent = nostr.Keys.generate();

  test('returns the owner pubkey for a valid auth tag', () {
    final tag = authTag(owner, agent.public);
    expect(
      verifiedOaOwnerPubkey(profile(agent, [tag])),
      owner.public.toLowerCase(),
    );
  });

  test('accepts valid conditions strings', () {
    final tag = authTag(owner, agent.public, conditions: 'kind=0');
    expect(
      verifiedOaOwnerPubkey(profile(agent, [tag])),
      owner.public.toLowerCase(),
    );
  });

  test('rejects a signature over a different agent pubkey', () {
    final otherAgent = nostr.Keys.generate();
    final tag = authTag(owner, otherAgent.public);
    expect(verifiedOaOwnerPubkey(profile(agent, [tag])), isNull);
  });

  test('rejects a tampered signature', () {
    final tag = authTag(owner, agent.public);
    final tampered = [...tag];
    tampered[3] = tampered[3].replaceRange(
      0,
      1,
      tampered[3][0] == '0' ? '1' : '0',
    );
    expect(verifiedOaOwnerPubkey(profile(agent, [tampered])), isNull);
  });

  test('rejects self-attestation', () {
    final tag = authTag(agent, agent.public);
    expect(verifiedOaOwnerPubkey(profile(agent, [tag])), isNull);
  });

  test('rejects malformed conditions', () {
    final tag = authTag(owner, agent.public, conditions: 'kind=abc');
    expect(verifiedOaOwnerPubkey(profile(agent, [tag])), isNull);
  });

  test('ignores unrelated tags', () {
    expect(
      verifiedOaOwnerPubkey(
        profile(agent, [
          ['p', owner.public],
        ]),
      ),
      isNull,
    );
  });
  test('rejects duplicate auth tags, including malformed companions', () {
    final tag = authTag(owner, agent.public);
    for (final duplicate in [
      tag,
      ['auth'],
      ['auth', 'invalid'],
    ]) {
      expect(verifiedOaOwnerPubkey(profile(agent, [tag, duplicate])), isNull);
      expect(verifiedOaOwnerPubkey(profile(agent, [duplicate, tag])), isNull);
    }
  });

  test('conditions evaluate the signed profile time, with strict bounds', () {
    for (final conditions in [
      '',
      'kind=0',
      'created_at>99&created_at<101',
      'created_at<4294967295',
    ]) {
      expect(
        verifiedOaOwnerPubkey(
          profile(agent, [
            authTag(owner, agent.public, conditions: conditions),
          ]),
        ),
        owner.public,
      );
    }
    for (final conditions in [
      'kind=1',
      'created_at>100',
      'created_at<100',
      'kind=65536',
      'kind=00',
      'kind=+0',
      'created_at<4294967296',
      'kind=0&',
      ' kind=0',
      'kind=0&kind=1',
    ]) {
      expect(
        verifiedOaOwnerPubkey(
          profile(agent, [
            authTag(owner, agent.public, conditions: conditions),
          ]),
        ),
        isNull,
        reason: conditions,
      );
    }
  });

  test(
    'rejects noncanonical owner/signature and invalid profile envelopes',
    () {
      final tag = authTag(owner, agent.public);
      for (final index in [1, 3]) {
        final uppercase = [...tag];
        uppercase[index] = uppercase[index].toUpperCase();
        expect(verifiedOaOwnerPubkey(profile(agent, [uppercase])), isNull);
      }
      final valid = profile(agent, [tag]);
      for (final patch in [
        {'content': 'forged'},
        {'created_at': 101},
        {'id': '0' * 64},
        {'sig': '0' * 128},
        {'pubkey': owner.public},
      ]) {
        expect(
          verifiedOaOwnerPubkey(
            NostrEvent.fromJson({...valid.toJson(), ...patch}),
          ),
          isNull,
        );
      }
      expect(verifiedOaOwnerPubkey(profile(agent, [tag], kind: 1)), isNull);
    },
  );
}
