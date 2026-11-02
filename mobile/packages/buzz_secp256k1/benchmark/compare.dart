// ignore_for_file: avoid_print
// Compares this package with the pure-Dart secp256k1 stack the mobile app
// uses today (nostr -> bip340, and pointycastle for ECDH).
//
//   dart run benchmark/compare.dart            (JIT)
//   dart build cli -t benchmark/compare.dart   (AOT, then run the bundle)
import 'dart:convert';
import 'dart:typed_data';

import 'package:buzz_secp256k1/buzz_secp256k1.dart' as native;
import 'package:crypto/crypto.dart' as crypto;
import 'package:nostr/nostr.dart' as nostr;
import 'package:pointycastle/ecc/curves/secp256k1.dart';

const _events = 200;
const _rounds = 5;

void main() {
  final keys = List.generate(8, (_) => nostr.Keys.generate());
  final payloads = [
    for (var i = 0; i < _events; i++)
      jsonEncode(
        nostr.Event.from(
          kind: 9,
          tags: [
            ['h', '3928fe05-df61-4b5d-b9c7-d623b9b10ea1'],
            ['p', keys[(i + 1) % keys.length].public],
          ],
          content: 'message $i ${'x' * (i % 300)}',
          secretKey: keys[i % keys.length].secret,
        ).toMap(),
      ),
  ];

  // Correctness gate before timing: both paths accept every event and reject
  // a tampered one.
  for (final p in payloads) {
    nostr.Event.fromJson(p);
    if (!_nativeVerifyEvent(p)) throw StateError('native rejected $p');
  }
  final tampered = payloads.first.replaceFirst('message 0', 'message X');
  if (_nativeVerifyEvent(tampered)) throw StateError('native accepted tamper');

  print('Event verify (parse + id hash + BIP-340), $_events events:');
  _bench('pure Dart  Event.fromJson', _events, () {
    for (final p in payloads) {
      nostr.Event.fromJson(p);
    }
  });
  _bench('libsecp256k1 FFI        ', _events, () {
    for (final p in payloads) {
      _nativeVerifyEvent(p);
    }
  });

  final msg = Uint8List.fromList(List.generate(32, (i) => i));
  final msgHex = _toHex(msg);
  final sk = keys.first.secret;
  final skBytes = _fromHex(sk);
  print('\nSign, $_events signatures:');
  _bench('pure Dart  Schnorr.sign ', _events, () {
    for (var i = 0; i < _events; i++) {
      nostr.Schnorr.sign(secretKey: sk, message: msgHex);
    }
  });
  _bench('libsecp256k1 FFI        ', _events, () {
    for (var i = 0; i < _events; i++) {
      native.schnorrSign(message: msg, secretKey: skBytes);
    }
  });

  final peer = keys[1].public;
  final peerBytes = _fromHex(peer);
  final expected = _pointycastleEcdh(sk, peer);
  if (_toHex(native.ecdhX(secretKey: skBytes, publicKey: peerBytes)) !=
      _toHex(expected)) {
    throw StateError('ECDH mismatch');
  }
  print('\nECDH (NIP-44 conversation key input), $_events:');
  _bench('pure Dart  pointycastle ', _events, () {
    for (var i = 0; i < _events; i++) {
      _pointycastleEcdh(sk, peer);
    }
  });
  _bench('libsecp256k1 FFI        ', _events, () {
    for (var i = 0; i < _events; i++) {
      native.ecdhX(secretKey: skBytes, publicKey: peerBytes);
    }
  });
}

bool _nativeVerifyEvent(String payload) {
  final e = nostr.Event.fromJson(payload, verify: false);
  final id = crypto.sha256
      .convert(
        utf8.encode(
          jsonEncode([0, e.pubkey, e.createdAt, e.kind, e.tags, e.content]),
        ),
      )
      .bytes;
  if (_toHex(Uint8List.fromList(id)) != e.id) return false;
  return native.schnorrVerify(
    signature: _fromHex(e.sig),
    message: Uint8List.fromList(id),
    publicKey: _fromHex(e.pubkey),
  );
}

// Same algorithm as mobile/lib/shared/crypto/ecdh.dart.
Uint8List _pointycastleEcdh(String skHex, String pubHex) {
  final params = ECCurve_secp256k1();
  final point = params.curve.decodePoint([0x02, ..._fromHex(pubHex)])!;
  final shared = (point * BigInt.parse(skHex, radix: 16))!;
  return _fromHex(shared.x!.toBigInteger()!.toRadixString(16).padLeft(64, '0'));
}

void _bench(String label, int ops, void Function() body) {
  body(); // warm up
  final times = <double>[];
  for (var r = 0; r < _rounds; r++) {
    final sw = Stopwatch()..start();
    body();
    times.add(sw.elapsedMicroseconds / ops);
  }
  times.sort();
  final median = times[times.length ~/ 2];
  print('  $label  ${(median / 1000).toStringAsFixed(3)} ms/op');
}

Uint8List _fromHex(String s) => Uint8List.fromList(
  List.generate(
    s.length ~/ 2,
    (i) => int.parse(s.substring(i * 2, i * 2 + 2), radix: 16),
  ),
);

String _toHex(Uint8List b) =>
    b.map((x) => x.toRadixString(16).padLeft(2, '0')).join();
