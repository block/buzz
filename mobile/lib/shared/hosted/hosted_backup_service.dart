import '../storage/serial_queue.dart';
import 'dart:async';
import 'dart:convert';

import 'package:crypto/crypto.dart';
import 'package:flutter_secure_storage/flutter_secure_storage.dart';

import '../storage/secure_storage.dart';

import 'account_identity_store.dart';
import 'builderlab_account.dart';
import 'hosted_environment.dart';
import 'hpke_backup.dart';

enum HostedBackupFailure {
  identityMismatch,
  storage,
  invalidResponse,
  rejected,
  unavailable,
}

/// A pending ciphertext is durable before its first upload. Uncertain uploads
/// replay that same envelope; only confirmed storage produces an admission receipt.
class HostedBackupService {
  HostedBackupService(
    this.account,
    this.identity, {
    HpkeBackup? hpke,
    FlutterSecureStorage? storage,
  }) : _hpke = hpke ?? HpkeBackup(),
       _storage = storage ?? accountSecureStorage;

  final BuilderlabAccountClient account;
  final AccountIdentityStore identity;
  final HpkeBackup _hpke;
  final FlutterSecureStorage _storage;
  static final _queue = SerialQueue();
  static String pendingKey(String subject, String pubkey) =>
      'buzz_backup_pending_v2_${sha256.convert(utf8.encode(jsonEncode([HostedEnvironment.origin.origin, subject, pubkey])))}';

  Future<void> backup({bool Function()? isCurrent}) {
    final result = _queue.run(() => _backup(isCurrent: isCurrent));
    return result;
  }

  Future<void> _backup({
    bool reenrolled = false,
    bool Function()? isCurrent,
  }) async {
    void checkCurrent() {
      if (isCurrent != null && !isCurrent()) throw AccountFailure.unavailable;
    }

    checkCurrent();
    final subject = account.account?.subject;
    final local = await identity.read();
    if (subject == null || local == null) {
      throw HostedBackupFailure.identityMismatch;
    }
    final backend = HostedEnvironment.origin.origin;
    final key = pendingKey(subject, local.pubkey);
    if (local.hasBackupFor(backend: backend, subject: subject)) return;
    void checkAccount() {
      if (account.account?.subject != subject) {
        throw HostedBackupFailure.identityMismatch;
      }
    }

    Map<String, dynamic>? envelope;
    try {
      final raw = await _storage.read(key: key);
      if (raw != null) {
        final record = jsonDecode(raw) as Map<String, dynamic>;
        if (record['backend'] != backend ||
            record['subject'] != subject ||
            record['pubkey'] != local.pubkey) {
          throw HostedBackupFailure.identityMismatch;
        }
        envelope = Map<String, dynamic>.from(record['envelope'] as Map);
        HpkeBackup.associatedData(envelope);
        if (envelope['nostr_pubkey'] != local.pubkey) {
          throw HostedBackupFailure.identityMismatch;
        }
      }
    } on HostedBackupFailure {
      rethrow;
    } catch (_) {
      throw HostedBackupFailure.storage;
    }
    checkAccount();
    checkCurrent();
    if (envelope == null) {
      final Map<String, dynamic> enrollment;
      try {
        enrollment = await account.post('/v1/buzz/nsec-backups/enrollment', {});
      } on AccountFailure catch (failure) {
        if (failure == AccountFailure.rejected) {
          throw HostedBackupFailure.unavailable;
        }
        rethrow;
      }
      checkAccount();
      if (enrollment['nostr_pubkey'] != local.pubkey) {
        throw HostedBackupFailure.identityMismatch;
      }
      envelope = await identity.withKeys(
        (keys) => _hpke.seal(enrollment: enrollment, secretHex: keys.secret),
      );
      checkAccount();
      final raw = jsonEncode({
        'backend': backend,
        'subject': subject,
        'pubkey': local.pubkey,
        'envelope': envelope,
      });
      try {
        await _storage.write(key: key, value: raw);
        if (await _storage.read(key: key) != raw) {
          throw HostedBackupFailure.storage;
        }
      } catch (_) {
        throw HostedBackupFailure.storage;
      }
    }
    checkAccount();
    checkCurrent();
    final pending = envelope!;
    final Map<String, dynamic> result;
    try {
      result = await account.post('/v1/buzz/nsec-backups', pending);
    } on AccountFailure catch (failure) {
      if (failure != AccountFailure.rejected) rethrow;
      checkAccount();
      checkCurrent();
      // Only a definitive rejection permits replacing the pending envelope.
      // A timeout/5xx/unknown response always preserves it for exact replay.
      if (reenrolled) throw HostedBackupFailure.rejected;
      try {
        await _storage.delete(key: key);
        if (await _storage.read(key: key) != null) {
          throw HostedBackupFailure.storage;
        }
      } catch (_) {
        throw HostedBackupFailure.storage;
      }
      return _backup(reenrolled: true, isCurrent: isCurrent);
    }
    checkAccount();
    if (result['stored'] != true ||
        result['backup_id'] != pending['backup_id']) {
      throw HostedBackupFailure.invalidResponse;
    }
    await identity.confirmBackup(
      BackupReceipt(
        backend: backend,
        subject: subject,
        pubkey: local.pubkey,
        backupId: pending['backup_id'] as String,
      ),
    );
    // Receipt is authoritative after this point; a failed cleanup cannot cause
    // another enrollment/upload on restart.
    try {
      await _storage.delete(key: key);
    } catch (_) {
      /* Retry is unnecessary once the receipt is durable. */
    }
  }
}
