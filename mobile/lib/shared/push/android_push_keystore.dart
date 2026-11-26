import 'dart:convert';

import 'package:crypto/crypto.dart' as crypto;
import 'package:flutter/services.dart';

const _keystoreChannel = MethodChannel('buzz/push_keystore');

/// The installation's Android Keystore key: P-256, sign-only, never
/// exportable, with the key attestation chain the gateway verifies.
class AndroidPushKey {
  const AndroidPushKey({required this.publicKey, required this.chain});

  /// The uncompressed SEC1 point, 65 bytes.
  final Uint8List publicKey;

  /// Leaf-first DER certificates.
  final List<Uint8List> chain;

  /// What the gateway calls this key: SHA-256 of [publicKey], base64.
  String get keyId => base64.encode(crypto.sha256.convert(publicKey).bytes);

  List<String> get chainBase64 => [
    for (final cert in chain) base64.encode(cert),
  ];
}

/// This phone cannot produce hardware key attestation, so it cannot enroll.
class AndroidPushAttestationUnavailable implements Exception {
  const AndroidPushAttestationUnavailable(this.reason);

  final String reason;

  @override
  String toString() => 'Key attestation is unavailable: $reason';
}

/// The Keystore boundary. One installation key exists at a time.
abstract interface class AndroidPushKeystore {
  /// The existing key, or null when none was generated.
  Future<AndroidPushKey?> current();

  /// Replaces any key with a new one whose attestation challenge is
  /// [attestationChallenge]. Throws [AndroidPushAttestationUnavailable] when
  /// the phone cannot attest.
  Future<AndroidPushKey> generate(Uint8List attestationChallenge);

  /// A base64 DER ECDSA signature over [data] (SHA-256 digest).
  Future<String> sign(Uint8List data);

  Future<void> delete();
}

class MethodChannelAndroidPushKeystore implements AndroidPushKeystore {
  const MethodChannelAndroidPushKeystore();

  @override
  Future<AndroidPushKey?> current() async =>
      _key(await _keystoreChannel.invokeMapMethod<String, Object?>('current'));

  @override
  Future<AndroidPushKey> generate(Uint8List attestationChallenge) async {
    try {
      final key = _key(
        await _keystoreChannel.invokeMapMethod<String, Object?>('generate', {
          'challenge': attestationChallenge,
        }),
      );
      if (key == null) {
        throw StateError('The Keystore returned no key.');
      }
      return key;
    } on PlatformException catch (error) {
      if (error.code == 'attestation_unavailable') {
        throw AndroidPushAttestationUnavailable(
          error.message ?? 'no attestation',
        );
      }
      rethrow;
    }
  }

  @override
  Future<String> sign(Uint8List data) async {
    final signature = await _keystoreChannel.invokeMethod<Uint8List>('sign', {
      'data': data,
    });
    if (signature == null) {
      throw StateError('The Keystore returned no signature.');
    }
    return base64.encode(signature);
  }

  @override
  Future<void> delete() => _keystoreChannel.invokeMethod<void>('delete');

  AndroidPushKey? _key(Map<String, Object?>? raw) {
    if (raw == null) return null;
    return AndroidPushKey(
      publicKey: raw['publicKey']! as Uint8List,
      chain: [
        for (final cert in raw['chain']! as List<Object?>) cert! as Uint8List,
      ],
    );
  }
}
