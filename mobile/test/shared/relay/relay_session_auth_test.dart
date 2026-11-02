import 'dart:convert';

import 'package:buzz/shared/crypto/event_signing.dart';
import 'package:buzz/shared/relay/relay_session.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:nostr/nostr.dart' as nostr;

void main() {
  final keys = nostr.Keys(
    'b7e151628aed2a6abf7158809cf4f3c762e7160f38b4da56a784d9045190cfef',
  );

  Map<String, dynamic> decode(String header) {
    expect(header, startsWith('Nostr '));
    return jsonDecode(utf8.decode(base64.decode(header.substring(6))))
        as Map<String, dynamic>;
  }

  String payloadTag(Map<String, dynamic> event) =>
      (event['tags'] as List).firstWhere((t) => t[0] == 'payload')[1] as String;

  test(
    'payload tag is the lowercase SHA-256 hex of the body (known vectors)',
    () {
      final vectors = {
        '': 'e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855',
        'abc':
            'ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad',
      };
      for (final entry in vectors.entries) {
        final header = buildNip98AuthHeader(
          method: 'post',
          url: 'https://relay.example/query',
          bodyBytes: utf8.encode(entry.key),
          nsec: keys.nsec,
        );
        final event = decode(header);
        expect(payloadTag(event), entry.value);
        expect(event['kind'], 27235);
        expect(event['pubkey'], keys.public);
        expect(
          (event['tags'] as List).firstWhere((t) => t[0] == 'method')[1],
          'POST',
        );
        expect(
          verifySchnorrHex(
            publicKey: event['pubkey'] as String,
            messageHex: event['id'] as String,
            signature: event['sig'] as String,
          ),
          isTrue,
        );
      }
    },
  );

  test('hashes bodies longer than one SHA-256 block identically', () {
    // 1,000,000 repetitions of "a" (FIPS 180 test vector).
    final header = buildNip98AuthHeader(
      method: 'POST',
      url: 'https://relay.example/query',
      bodyBytes: List.filled(1000000, 0x61),
      nsec: keys.nsec,
    );
    expect(
      payloadTag(decode(header)),
      'cdc76e5c9914fb9281a1c7e284d73e67f1809a48a497200e046d39ccc7112cd0',
    );
  });
}
