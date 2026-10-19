import 'dart:convert';
import 'dart:typed_data';

import 'package:flutter/services.dart';
import 'package:nostr/nostr.dart' as nostr;

/// Never include input, plaintext, or native exception details in this error.
enum HpkeBackupFailure { invalidEnrollment, identityMismatch, unavailable }

/// Seal only enrollment obtained from the fixed-origin authenticated account
/// client. Never pass recipient material from a relay, deep link or QR payload.
/// Persist the returned envelope BEFORE upload; retry its exact backup_id and
/// ciphertext instead of enrolling/sealing again after uncertain responses.
class HpkeBackup {
  static const _channel = MethodChannel('buzz/hosted_hpke');

  Future<Map<String, dynamic>> seal({
    required Map<String, dynamic> enrollment,
    required String secretHex,
  }) async {
    try {
      final pubkey = nostr.Keys(secretHex).public;
      if (enrollment['nostr_pubkey'] != pubkey) {
        throw HpkeBackupFailure.identityMismatch;
      }
      final aad = associatedData(enrollment);
      final recipient = _decode(enrollment['recipient_public_key'] as String);
      if (recipient.length != 65 || recipient.first != 4) {
        throw HpkeBackupFailure.invalidEnrollment;
      }
      final plaintext = _hex(secretHex, 32);
      final Map<dynamic, dynamic>? sealed;
      try {
        sealed = await _channel.invokeMapMethod<dynamic, dynamic>('seal', {
          'recipient': recipient,
          'plaintext': plaintext,
          'aad': aad,
        });
      } finally {
        plaintext.fillRange(0, plaintext.length, 0);
      }
      final enc = sealed?['enc'];
      final ciphertext = sealed?['ciphertext'];
      if (enc is! Uint8List ||
          enc.length != 65 ||
          enc.first != 4 ||
          ciphertext is! Uint8List ||
          ciphertext.length != 48) {
        throw HpkeBackupFailure.unavailable;
      }
      return {
        for (final field in [
          'version',
          'suite',
          'recipient_key_id',
          'service_namespace',
          'owner_id',
          'backup_id',
          'nostr_pubkey',
        ])
          field: enrollment[field],
        'enc': _encode(enc),
        'ciphertext': _encode(ciphertext),
      };
    } on HpkeBackupFailure {
      rethrow;
    } on PlatformException {
      throw HpkeBackupFailure.unavailable;
    } on MissingPluginException {
      throw HpkeBackupFailure.unavailable;
    } catch (_) {
      throw HpkeBackupFailure.invalidEnrollment;
    }
  }

  /// Exact v1 wire framing shared with desktop and the recovery service.
  static Uint8List associatedData(Map<String, dynamic> enrollment) {
    try {
      final suite = enrollment['suite'] as Map;
      if (enrollment['version'] != 1 ||
          suite['kem_id'] != 16 ||
          suite['kdf_id'] != 1 ||
          suite['aead_id'] != 2) {
        throw HpkeBackupFailure.invalidEnrollment;
      }
      final bytes = BytesBuilder(copy: false)
        ..add(utf8.encode('buzz/nsec-backup/aad/v1'))
        ..add([1, 0, 16, 0, 1, 0, 2]);
      for (final field in [
        'recipient_key_id',
        'service_namespace',
        'owner_id',
      ]) {
        final value = enrollment[field] as String;
        final encoded = utf8.encode(value);
        if (encoded.isEmpty ||
            encoded.length > 255 ||
            value.runes.any((r) => r < 32 || (r >= 127 && r <= 159))) {
          throw HpkeBackupFailure.invalidEnrollment;
        }
        bytes.add([0, encoded.length]);
        bytes.add(encoded);
      }
      final id = enrollment['backup_id'] as String;
      if (!RegExp(
        r'^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$',
      ).hasMatch(id)) {
        throw HpkeBackupFailure.invalidEnrollment;
      }
      bytes.add(_hex(id.replaceAll('-', ''), 16));
      bytes.add(_hex(enrollment['nostr_pubkey'] as String, 32));
      return bytes.toBytes();
    } on HpkeBackupFailure {
      rethrow;
    } catch (_) {
      throw HpkeBackupFailure.invalidEnrollment;
    }
  }

  static Uint8List _hex(String value, int length) {
    if (value.length != length * 2 || !RegExp(r'^[0-9a-f]+$').hasMatch(value)) {
      throw HpkeBackupFailure.invalidEnrollment;
    }
    return Uint8List.fromList(
      List.generate(
        length,
        (i) => int.parse(value.substring(i * 2, i * 2 + 2), radix: 16),
      ),
    );
  }

  static String _encode(List<int> bytes) =>
      base64UrlEncode(bytes).replaceAll('=', '');
  static Uint8List _decode(String value) {
    final decoded = base64Url.decode(base64Url.normalize(value));
    if (_encode(decoded) != value) throw HpkeBackupFailure.invalidEnrollment;
    return decoded;
  }
}
