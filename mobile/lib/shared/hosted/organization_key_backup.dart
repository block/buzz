import 'dart:async';

import 'package:flutter/foundation.dart';
import 'package:flutter/services.dart';
import 'package:flutter_secure_storage/flutter_secure_storage.dart';
import 'package:nostr/nostr.dart' as nostr;

import '../storage/secure_storage.dart';
import 'account_identity_store.dart';
import 'builderlab_account.dart';
import 'hosted_backup_service.dart';
import 'hpke_backup.dart';

/// Backs up an organization relay's key to the organization account.
///
/// The tenant binds a member's existing key when it issues the first relay
/// assertion for it, so [HostedRelayAuth] calls [afterAssertion] each time it
/// issues one. The account's backup listing is authoritative: a listed key
/// needs nothing more, and an unlisted one is sealed and uploaded through
/// [HostedBackupService], which keeps its pending envelope for exact replay.
///
/// A tenant without backups answers 404, and a definitive refusal cannot
/// succeed unchanged, so either ends attempts for that account and key until
/// the app restarts. Any other failure is retried at a later assertion, after
/// a delay that doubles from [firstRetryDelay] up to [maxRetryDelay]. The
/// tenant's listing is the durable record: every launch, and every assertion
/// renewal in a long-running one, checks it again until the key is listed.
class OrganizationKeyBackup {
  OrganizationKeyBackup(
    this.account, {
    HpkeBackup? hpke,
    FlutterSecureStorage? storage,
    Future<bool> Function()? supportsSealing,
    DateTime Function()? now,
  }) : _hpke = hpke,
       _storage = storage,
       _supportsSealing = supportsSealing ?? HpkeBackup.supportsSealing,
       _now = now ?? DateTime.now;

  final BuilderlabAccountClient account;
  final HpkeBackup? _hpke;
  final FlutterSecureStorage? _storage;
  final Future<bool> Function() _supportsSealing;
  final DateTime Function() _now;

  /// Account and key pairs that are backed up, or that cannot be here.
  final _settled = <String>{};
  final _running = <String, Future<void>>{};
  final _failures = <String, int>{};
  final _retryAt = <String, DateTime>{};

  /// A relay refusal re-issues an assertion at once, so retries wait between
  /// attempts instead of following every assertion.
  static const firstRetryDelay = Duration(minutes: 1);
  static const maxRetryDelay = Duration(hours: 1);

  /// The latest attempt's failure, cleared by a completed attempt.
  @visibleForTesting
  Enum? lastFailure;

  /// Completes without error. A failure is recorded and retried at the first
  /// assertion after its retry delay.
  Future<void> afterAssertion(String nsec, String pubkey) {
    final subject = account.account?.subject;
    if (subject == null) return Future.value();
    final scope = '$subject\n$pubkey';
    if (_settled.contains(scope)) return Future.value();
    final retryAt = _retryAt[scope];
    if (retryAt != null && _now().isBefore(retryAt)) return Future.value();
    return _running[scope] ??= _attempt(subject, nsec, pubkey, scope)
        .whenComplete(() {
          // Not `=> remove`: whenComplete would await the removed future, itself.
          _running.remove(scope);
        });
  }

  Future<void> _attempt(
    String subject,
    String nsec,
    String pubkey,
    String scope,
  ) async {
    try {
      // iOS 16 can recover backups but cannot seal new ones.
      if (await _canSeal()) {
        await _backUp(subject, nsec, pubkey);
      }
      _settled.add(scope);
      _failures.remove(scope);
      _retryAt.remove(scope);
      lastFailure = null;
    } on AccountFailure catch (failure) {
      _failed(scope, failure);
    } on HostedBackupFailure catch (failure) {
      _failed(scope, failure);
    } on HpkeBackupFailure catch (failure) {
      _failed(scope, failure);
    } on AccountIdentityFailure catch (failure) {
      _failed(scope, failure);
    }
  }

  Future<bool> _canSeal() async {
    try {
      return await _supportsSealing();
    } on PlatformException {
      throw HpkeBackupFailure.unavailable;
    } on MissingPluginException {
      throw HpkeBackupFailure.unavailable;
    }
  }

  void _failed(String scope, Enum failure) {
    debugPrint('Organization key backup failed: ${failure.name}');
    lastFailure = failure;
    // A tenant without backups (404) and other definitive refusals cannot
    // succeed unchanged.
    if (const {
      AccountFailure.rejected,
      HostedBackupFailure.unavailable,
      HostedBackupFailure.rejected,
      HpkeBackupFailure.invalidEnrollment,
    }.contains(failure)) {
      _settled.add(scope);
      return;
    }
    final failures = (_failures[scope] ?? 0) + 1;
    _failures[scope] = failures;
    final delay = firstRetryDelay * (1 << (failures - 1).clamp(0, 6));
    _retryAt[scope] = _now().add(delay > maxRetryDelay ? maxRetryDelay : delay);
  }

  Future<void> _backUp(String subject, String nsec, String pubkey) async {
    final status = await account.post(
      '/v1/buzz/nsec-backups/status',
      {},
      expectedSubject: subject,
    );
    final listed = status['backup_nostr_pubkeys'];
    if (status['error'] != null || listed is! List) {
      throw AccountFailure.invalidResponse;
    }
    if (listed.contains(pubkey)) {
      // An upload whose response was lost may have stored this backup.
      try {
        await (_storage ?? accountSecureStorage).delete(
          key: HostedBackupService.pendingKey(
            account.accountOrigin.origin,
            subject,
            pubkey,
          ),
        );
      } catch (_) {
        throw HostedBackupFailure.storage;
      }
      return;
    }
    // The assertion that started this attempt bound this key, so any other
    // binding means the account changed underneath it.
    if (status['current_nostr_pubkey'] != pubkey) {
      throw HostedBackupFailure.identityMismatch;
    }
    // The service checks the subject before each request. Its requests share
    // the organization client's queue, so a sign-out waits for one in flight.
    await HostedBackupService(
      account,
      _CommunityKey(nsec, pubkey),
      hpke: _hpke,
      storage: _storage,
    ).backup();
  }
}

/// A community's own key. Completion is read from the account's backup
/// listing, so the receipt only needs to outlive one upload.
class _CommunityKey implements BackupIdentity {
  _CommunityKey(this._nsec, this._pubkey);

  final String _nsec;
  final String _pubkey;
  BackupReceipt? _receipt;

  @override
  Future<AccountIdentitySnapshot?> read() async =>
      AccountIdentitySnapshot(_pubkey, _receipt);

  @override
  Future<T> withKeys<T>(Future<T> Function(nostr.Keys keys) operation) =>
      operation(AccountIdentityStore.keysForNsec(_nsec));

  @override
  Future<void> confirmBackup(BackupReceipt receipt) async {
    if (receipt.pubkey != _pubkey) throw AccountIdentityFailure.mismatch;
    _receipt = receipt;
  }
}
