import 'dart:typed_data';

import 'package:pointycastle/api.dart';
import 'package:pointycastle/digests/sha256.dart';
import 'package:pointycastle/macs/hmac.dart';

/// HKDF-SHA256 Extract: PRK = HMAC-SHA256(salt, ikm).
Uint8List hkdfExtract(Uint8List salt, Uint8List ikm) {
  final hmac = HMac(SHA256Digest(), 64);
  hmac.init(KeyParameter(salt.isEmpty ? Uint8List(32) : salt));
  final out = Uint8List(32);
  hmac.update(ikm, 0, ikm.length);
  hmac.doFinal(out, 0);
  return out;
}

/// HKDF-SHA256 Expand: OKM = HKDF-Expand(prk, info, length).
Uint8List hkdfExpand(Uint8List prk, Uint8List info, int length) {
  final hmac = HMac(SHA256Digest(), 64);
  hmac.init(KeyParameter(prk));

  final n = (length + 31) ~/ 32; // ceil(length / hashLen)
  final okm = Uint8List(n * 32);
  var prev = Uint8List(0);

  for (var i = 1; i <= n; i++) {
    hmac.reset();
    hmac.update(prev, 0, prev.length);
    hmac.update(info, 0, info.length);
    final counter = Uint8List.fromList([i]);
    hmac.update(counter, 0, 1);
    prev = Uint8List(32);
    hmac.doFinal(prev, 0);
    okm.setRange((i - 1) * 32, i * 32, prev);
  }

  return Uint8List.sublistView(okm, 0, length);
}

/// Convenience: HKDF-SHA256(ikm, salt, info, length).
Uint8List hkdf(Uint8List ikm, Uint8List salt, Uint8List info, int length) {
  final prk = hkdfExtract(salt, ikm);
  return hkdfExpand(prk, info, length);
}
