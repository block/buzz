import '../storage/serial_queue.dart';
import 'dart:async';
import 'dart:convert';

import 'package:flutter_secure_storage/flutter_secure_storage.dart';

import 'package:nostr/nostr.dart' as nostr;

import '../storage/secure_storage.dart';

import 'hosted_environment.dart';

/// Safe failures: never include a secret or the storage/plugin exception.
enum AccountIdentityFailure {
  unreadable,
  invalidKey,
  occupied,
  absent,
  mismatch,
}

/// Evidence from a successful backup/recovery, not merely an attempted upload.
class BackupReceipt {
  BackupReceipt({
    required this.backend,
    required this.subject,
    required this.pubkey,
    required this.backupId,
  }) {
    final uri = Uri.tryParse(backend);
    if (uri == null ||
        uri.scheme != 'https' ||
        uri.host.isEmpty ||
        uri.origin != backend ||
        subject.isEmpty ||
        backupId.isEmpty ||
        !RegExp(r'^[0-9a-f]{64}$').hasMatch(pubkey)) {
      throw AccountIdentityFailure.mismatch;
    }
  }

  final String backend;
  final String subject;
  final String pubkey;
  final String backupId;

  Map<String, String> _json() => {
    'backend': backend,
    'subject': subject,
    'pubkey': pubkey,
    'backup_id': backupId,
  };

  static BackupReceipt _decode(dynamic value) => BackupReceipt(
    backend: value['backend'] as String,
    subject: value['subject'] as String,
    pubkey: value['pubkey'] as String,
    backupId: value['backup_id'] as String,
  );
}

/// Public facts only. Signing material never enters observable UI state.
class AccountIdentitySnapshot {
  const AccountIdentitySnapshot(this.pubkey, this.backup);
  final String pubkey;
  final BackupReceipt? backup;

  bool hasBackupFor({required String backend, required String subject}) =>
      // Retired walkthrough receipts must never authorize real admission.
      !(backup?.backupId.startsWith('debug-simulated-backup:') ?? false) &&
      backup?.backend == backend &&
      backup?.subject == subject &&
      backup?.pubkey == pubkey;
}

/// Single create-only AccountKey slot per configured backend. Exact-key retries
/// are idempotent; they never discard or replace an existing backup receipt. Does not choose or mutate legacy
/// community identities. Migration must explicitly resolve a candidate first.
///
/// Key and recovered receipt share one secure record, so restart cannot observe
/// a saved recovered key without its receipt. All instances serialize within the
/// app isolate. Native extensions/other isolates must not write this slot:
/// FlutterSecureStorage does not provide a cross-process compare-and-set.
class AccountIdentityStore {
  AccountIdentityStore({FlutterSecureStorage? storage})
    : _storage = storage ?? accountSecureStorage;

  static final storageKey =
      'buzz_account_identity_${HostedEnvironment.origin.host.replaceAll('.', '_')}_v1';
  static final _queue = SerialQueue();
  final FlutterSecureStorage _storage;

  Future<T> _serialized<T>(Future<T> Function() operation) {
    final result = _queue.run(() => operation());
    return result;
  }

  Future<AccountIdentitySnapshot?> read() => _serialized(() async {
    final record = await _read();
    return record?.snapshot;
  });

  Future<AccountIdentitySnapshot> create(
    String nsec, {
    BackupReceipt? recoveredBackup,
    void Function()? checkCurrent,
  }) => _serialized(() async {
    checkCurrent?.call();
    // Read before parsing/generating any candidate: corrupt != absent.
    final existing = await _read();
    final keys = _keys(nsec);
    if (existing != null) {
      if (existing.pubkey != keys.public) throw AccountIdentityFailure.occupied;
      if (recoveredBackup == null) return existing.snapshot;
      if (existing.backup != null) {
        if (jsonEncode(existing.backup!._json()) !=
            jsonEncode(recoveredBackup._json())) {
          throw AccountIdentityFailure.mismatch;
        }
        return existing.snapshot;
      }
    }
    if (recoveredBackup != null && recoveredBackup.pubkey != keys.public) {
      throw AccountIdentityFailure.mismatch;
    }
    checkCurrent?.call();
    final record = _Record(nsec, keys.public, recoveredBackup);
    await _write(record);
    return record.snapshot;
  });

  /// Authorization runs under the identity writer lock and must not reenter it.
  /// A completed but unacknowledged write is reused on retry, never regenerated.
  Future<AccountIdentitySnapshot?> generateIfAbsent({
    required Future<bool> Function() authorize,
  }) => _serialized(() async {
    final existing = await _read();
    if (existing != null) return existing.snapshot;
    if (!await authorize()) return null;
    final keys = nostr.Keys.generate();
    final record = _Record(keys.nsec, keys.public, null);
    await _write(record);
    return record.snapshot;
  });

  Future<AccountIdentitySnapshot> confirmBackup(BackupReceipt receipt) =>
      _serialized(() async {
        final record = await _read();
        if (record == null) throw AccountIdentityFailure.absent;
        if (record.pubkey != receipt.pubkey) {
          throw AccountIdentityFailure.mismatch;
        }
        final updated = _Record(record.nsec, record.pubkey, receipt);
        await _write(updated);
        return updated.snapshot;
      });

  /// Service-only signing/export boundary. Do not return keys from this closure
  /// to a provider/widget. Avoid reentering this store inside the callback.
  Future<T> withKeys<T>(Future<T> Function(nostr.Keys keys) operation) =>
      _serialized(() async {
        final record = await _read();
        if (record == null) throw AccountIdentityFailure.absent;
        return operation(_keys(record.nsec));
      });

  Future<_Record?> _read() async {
    try {
      final raw = await _storage.read(key: storageKey);
      if (raw == null) return null;
      final value = jsonDecode(raw) as Map<String, dynamic>;
      if (value['version'] != 1) throw AccountIdentityFailure.unreadable;
      final nsec = value['nsec'] as String;
      final pubkey = _keys(nsec).public;
      if (value['pubkey'] != pubkey) throw AccountIdentityFailure.unreadable;
      final receipt = value['backup'] == null
          ? null
          : BackupReceipt._decode(value['backup']);
      if (receipt != null && receipt.pubkey != pubkey) {
        throw AccountIdentityFailure.unreadable;
      }
      return _Record(nsec, pubkey, receipt);
    } catch (_) {
      throw AccountIdentityFailure.unreadable;
    }
  }

  Future<void> _write(_Record record) async {
    final raw = jsonEncode({
      'version': 1,
      'nsec': record.nsec,
      'pubkey': record.pubkey,
      'backup': record.backup?._json(),
    });
    try {
      await _storage.write(key: storageKey, value: raw);
      if (await _storage.read(key: storageKey) != raw) {
        throw AccountIdentityFailure.unreadable;
      }
    } catch (_) {
      // A write may have completed: caller must reread, never replace/recreate.
      throw AccountIdentityFailure.unreadable;
    }
  }

  /// Validate a candidate without exposing signing material to the UI.
  static String publicKeyForNsec(String nsec) => _keys(nsec).public;

  /// Rejects invalid secp256k1 scalars before constructing a signing key.
  static nostr.Keys keysForHex(String hex) {
    if (!RegExp(r'^[0-9a-fA-F]{64}$').hasMatch(hex)) {
      throw AccountIdentityFailure.invalidKey;
    }
    final scalar = BigInt.parse(hex, radix: 16);
    final order = BigInt.parse(
      'fffffffffffffffffffffffffffffffebaaedce6af48a03bbfd25e8cd0364141',
      radix: 16,
    );
    if (scalar <= BigInt.zero || scalar >= order) {
      throw AccountIdentityFailure.invalidKey;
    }
    return nostr.Keys(hex);
  }

  static nostr.Keys _keys(String nsec) {
    try {
      final decoded = nostr.Nip19.decode(payload: nsec);
      if (decoded.prefix != nostr.Nip19Prefix.nsec ||
          !RegExp(r'^[0-9a-fA-F]{64}$').hasMatch(decoded.data)) {
        throw AccountIdentityFailure.invalidKey;
      }
      return keysForHex(decoded.data);
    } catch (_) {
      throw AccountIdentityFailure.invalidKey;
    }
  }
}

class _Record {
  const _Record(this.nsec, this.pubkey, this.backup);
  final String nsec;
  final String pubkey;
  final BackupReceipt? backup;
  AccountIdentitySnapshot get snapshot =>
      AccountIdentitySnapshot(pubkey, backup);
}
