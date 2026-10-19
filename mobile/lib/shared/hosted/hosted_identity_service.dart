import 'dart:convert';

import 'package:nostr/nostr.dart' as nostr;

import 'account_identity_store.dart';
import 'builderlab_account.dart';
import 'device_identity_source.dart';

enum HostedIdentityState {
  ready,
  needsSelection,
  needsBinding,
  needsBackup,
  mismatch,
  recoveryBlocked,
  needsAuthoritativeAbsence,
}

class HostedIdentityResult {
  const HostedIdentityResult(
    this.state, {
    this.pubkey,
    this.choices = const [],
  });
  final HostedIdentityState state;
  final String? pubkey;
  final List<DeviceIdentityChoice> choices;
}

/// Resolves identity safely. Generation requires a separately verified
/// complete absence contract and explicit legacy-key reconciliation; neither a
/// failed recovery nor an unbound account proves there is no prior identity.
class HostedIdentityService {
  HostedIdentityService({
    required this.account,
    required this.store,
    DeviceIdentitySource? deviceIdentities,
  }) : deviceIdentities = deviceIdentities ?? DeviceIdentitySource();

  final DeviceIdentitySource deviceIdentities;
  final BuilderlabAccountClient account;
  final AccountIdentityStore store;

  Future<HostedIdentityResult> resolve({
    bool Function()? isCurrent,
    String? selectedPubkey,
  }) => _resolve(isCurrent, selectedPubkey);

  Future<HostedIdentityResult> _resolve(
    bool Function()? isCurrent,
    String? selectedPubkey,
  ) async {
    void checkCurrent() {
      if (isCurrent != null && !isCurrent()) throw AccountFailure.unavailable;
    }

    checkCurrent();
    final subject = account.account?.subject;
    if (subject == null) throw AccountFailure.signedOut;
    final store = this.store.forSubject(subject);
    final local = await store.read();
    final response = await account.currentIdentity();
    _sameAccount(subject);
    if (response['error'] != null) throw AccountFailure.invalidResponse;
    final identity = response['identity'];
    final String? bound;
    if (identity == null) {
      bound = null; // The current API omits the optional unbound identity.
    } else if (identity is Map<String, dynamic>) {
      bound = _pubkey(identity['pubkey_hex']);
    } else {
      throw AccountFailure.invalidResponse;
    }
    if (local != null && bound != null) return _result(local, bound, subject);

    final status = await account.post('/v1/buzz/nsec-backups/status', {});
    _sameAccount(subject);
    checkCurrent();
    final listed = status['backup_nostr_pubkeys'];
    if (status['error'] != null || listed is! List) {
      throw AccountFailure.invalidResponse;
    }
    final statusBound = status['current_nostr_pubkey'] == null
        ? null
        : _pubkey(status['current_nostr_pubkey']);
    final statusKeys = listed.map(_pubkey).toSet();
    if (statusBound != bound) throw AccountFailure.invalidResponse;
    if (local != null) {
      // A previously chosen recovery key may still await binding after a
      // restart. Its account-scoped receipt and current backup listing preserve
      // that choice even when the account also retains older identities.
      final confirmedSelection =
          statusKeys.contains(local.pubkey) &&
          local.hasBackupFor(
            backend: BuilderlabAccountClient.origin.origin,
            subject: subject,
          );
      if (statusKeys.any((key) => key != local.pubkey) && !confirmedSelection) {
        return const HostedIdentityResult(HostedIdentityState.mismatch);
      }
      return _result(local, bound, subject);
    }
    final device = await deviceIdentities.choices();
    final savedAccounts = await store.identities();
    final choices = [
      ...device,
      for (final saved in savedAccounts)
        if (!device.any((item) => item.pubkey == saved.pubkey))
          DeviceIdentityChoice(saved.pubkey, 'Saved identity'),
    ];
    _sameAccount(subject);
    checkCurrent();
    // The authoritative bound key already on this device needs no remote
    // decryption, even when the account retains backups of older identities.
    final matchingDevice =
        bound != null && choices.any((item) => item.pubkey == bound);
    if ((statusKeys.isEmpty && bound == null) || matchingDevice) {
      final eligible = choices
          .where((item) => bound == null || item.pubkey == bound)
          .toList();
      if (eligible.isNotEmpty) {
        final chosen =
            selectedPubkey ??
            (bound != null && eligible.length == 1
                ? eligible.single.pubkey
                : null);
        if (!eligible.any((item) => item.pubkey == chosen)) {
          return HostedIdentityResult(
            HostedIdentityState.needsSelection,
            choices: eligible,
          );
        }
        void checkSelection() {
          _sameAccount(subject);
          checkCurrent();
        }

        final saved = device.any((item) => item.pubkey == chosen)
            ? await deviceIdentities.adopt(
                chosen!,
                store,
                checkCurrent: checkSelection,
              )
            : await store.adoptSaved(chosen!, checkCurrent: checkSelection);
        return _result(saved, bound, subject);
      }
    }
    if (statusBound == null && statusKeys.isEmpty) {
      final generated = await store.generateIfAbsent(
        authorize: () async {
          _sameAccount(subject);
          checkCurrent();
          final reconciled = (await deviceIdentities.choices()).isEmpty;
          _sameAccount(subject);
          checkCurrent();
          return reconciled;
        },
      );
      _sameAccount(subject);
      checkCurrent();
      return generated == null
          ? const HostedIdentityResult(
              HostedIdentityState.needsAuthoritativeAbsence,
            )
          : _result(generated, bound, subject);
    }

    final recovered = await account.post('/v1/buzz/nsec-backups/recovery', {});
    _sameAccount(subject);
    final backups = recovered['backups'];
    final failed = recovered['failed_backup_ids'];
    if (recovered['error'] != null ||
        backups is! List ||
        failed is! List ||
        failed.any((id) => id is! String)) {
      throw AccountFailure.invalidResponse;
    }
    if (failed.isNotEmpty) {
      return const HostedIdentityResult(HostedIdentityState.recoveryBlocked);
    }
    if (backups.isEmpty) {
      return HostedIdentityResult(
        bound == null
            ? HostedIdentityState.needsAuthoritativeAbsence
            : HostedIdentityState.recoveryBlocked,
      );
    }
    // Validate the entire result before storing anything. Never trust its
    // advertised pubkey without deriving it from the actual secret.
    final candidates = <String, ({nostr.Keys keys, String backupId})>{};
    try {
      for (final backup in backups) {
        if (backup is! Map<String, dynamic>) throw const FormatException();
        final secret = backup['secret_key'];
        final backupId = backup['backup_id'];
        if (secret is! String ||
            !RegExp(r'^[A-Za-z0-9_-]{43}$').hasMatch(secret) ||
            backupId is! String ||
            backupId.isEmpty) {
          throw const FormatException();
        }
        final bytes = base64Url.decode(base64Url.normalize(secret));
        if (bytes.length != 32) throw const FormatException();
        final hex = bytes
            .map((b) => b.toRadixString(16).padLeft(2, '0'))
            .join();
        final keys = AccountIdentityStore.keysForHex(hex);
        if (keys.public != _pubkey(backup['nostr_pubkey'])) {
          throw const FormatException();
        }
        candidates.putIfAbsent(
          keys.public,
          () => (keys: keys, backupId: backupId),
        );
      }
    } catch (_) {
      throw AccountFailure.invalidResponse;
    }
    if (bound == null &&
        (statusKeys.length != candidates.length ||
            !statusKeys.every(candidates.containsKey))) {
      return const HostedIdentityResult(HostedIdentityState.recoveryBlocked);
    }
    // An account binding is authoritative even when older backups contain
    // other keys. Without a binding, distinct keys require an explicit choice;
    // never let response ordering choose the user's identity.
    final chosen =
        bound ??
        selectedPubkey ??
        (candidates.length == 1 ? candidates.keys.single : null);
    final candidate = candidates[chosen];
    if (candidate == null) {
      if (bound != null) {
        return const HostedIdentityResult(HostedIdentityState.mismatch);
      }
      return HostedIdentityResult(
        HostedIdentityState.needsSelection,
        choices: [
          for (final pubkey in candidates.keys)
            DeviceIdentityChoice(
              pubkey,
              'Use Builderlab identity',
              fromCloud: true,
            ),
        ],
      );
    }
    if (device.isNotEmpty &&
        !device.any((item) => item.pubkey == candidate.keys.public) &&
        selectedPubkey != candidate.keys.public) {
      return HostedIdentityResult(
        HostedIdentityState.needsSelection,
        choices: [
          DeviceIdentityChoice(
            candidate.keys.public,
            'Use Builderlab identity',
            fromCloud: true,
          ),
        ],
      );
    }
    _sameAccount(subject);
    checkCurrent();
    final saved = await store.create(
      candidate.keys.nsec,
      checkCurrent: () {
        _sameAccount(subject);
        checkCurrent();
      },
      recoveredBackup: BackupReceipt(
        backend: BuilderlabAccountClient.origin.origin,
        subject: subject,
        pubkey: candidate.keys.public,
        backupId: candidate.backupId,
      ),
    );
    _sameAccount(subject);
    checkCurrent();
    return _result(saved, bound, subject);
  }

  HostedIdentityResult _result(
    AccountIdentitySnapshot local,
    String? bound,
    String subject,
  ) {
    _sameAccount(subject);
    if (bound != null && bound != local.pubkey) {
      return HostedIdentityResult(
        HostedIdentityState.mismatch,
        pubkey: local.pubkey,
      );
    }
    if (bound == null) {
      return HostedIdentityResult(
        HostedIdentityState.needsBinding,
        pubkey: local.pubkey,
      );
    }
    return HostedIdentityResult(
      local.hasBackupFor(
            backend: BuilderlabAccountClient.origin.origin,
            subject: subject,
          )
          ? HostedIdentityState.ready
          : HostedIdentityState.needsBackup,
      pubkey: local.pubkey,
    );
  }

  void _sameAccount(String subject) {
    if (account.account?.subject != subject) {
      throw AccountFailure.differentAccount;
    }
  }

  static String _pubkey(dynamic value) {
    if (value is! String || !RegExp(r'^[a-fA-F0-9]{64}$').hasMatch(value)) {
      throw AccountFailure.invalidResponse;
    }
    return value.toLowerCase();
  }
}
