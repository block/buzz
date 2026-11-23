part of 'hosted_flow.dart';

/// Consent before reserving an existing identity or exporting its secret.
extension HostedFlowBackupConsent on HostedFlow {
  bool get bindingOnly => _target is NostrBindDeepLink;

  String? get backupAccountEmail {
    if (_pendingBackup?.subject != account.account?.subject) return null;
    final email = account.account?.email?.trim();
    return email == null || email.isEmpty ? null : email;
  }

  void _presentBackupConsent(
    String subject,
    String pubkey, {
    String? importNsec,
  }) {
    _pendingBackup = (subject: subject, pubkey: pubkey);
    _pendingImport = importNsec;
    _set(HostedFlowStage.confirmBackup);
  }

  /// Approval is bound to the displayed account, key and live presentation.
  Future<void> Function()? get confirmBackup {
    final approved = _pendingBackup;
    final presentedGeneration = _generation;
    if (stage != HostedFlowStage.confirmBackup ||
        approved == null ||
        backupAccountEmail == null) {
      return null;
    }
    return () {
      if (_disposed ||
          presentedGeneration != _generation ||
          stage != HostedFlowStage.confirmBackup ||
          _pendingBackup != approved ||
          account.account?.subject != approved.subject) {
        return Future.value();
      }
      final importNsec = _pendingImport;
      final generation = _nextGeneration();
      _set(HostedFlowStage.working);
      return _enqueue(
        () => _continue(
          generation,
          selectedPubkey: approved.pubkey,
          approvedBackup: approved,
          importNsec: importNsec,
        ),
      );
    };
  }

  /// Create explicitly from setup, or beside the saved identities offered.
  Future<void> createIdentity() {
    if ((_target == null && !accountOnly) ||
        !canCreateIdentity ||
        (stage != HostedFlowStage.setupIdentity &&
            stage != HostedFlowStage.chooseIdentity)) {
      return Future.value();
    }
    final offered = {for (final item in identityChoices) item.pubkey};
    final generation = _nextGeneration();
    _set(HostedFlowStage.working);
    return _enqueue(
      () => _continue(generation, createNew: true, offered: offered),
    );
  }

  /// Keep an imported key tentative until the confirmation is accepted. Only
  /// its public key is exposed to widgets; cancellation drops the private input.
  Future<void> importIdentity(String nsec) {
    final generation = _generation;
    final subject = account.account?.subject;
    if (stage != HostedFlowStage.setupIdentity || subject == null) {
      throw AccountFailure.unavailable;
    }
    return _queue.run(() async {
      bool current() =>
          !_disposed &&
          generation == _generation &&
          account.account?.subject == subject;
      if (!current()) throw AccountFailure.unavailable;
      final resolved = await identity.resolve(
        isCurrent: current,
        importNsec: nsec,
        canAdopt: (_) => false,
      );
      if (!current()) throw AccountFailure.unavailable;
      if (resolved.state == HostedIdentityState.needsConsent) {
        _presentBackupConsent(subject, resolved.pubkey!, importNsec: nsec);
      }
    });
  }

  Future<bool> _allowBackup(
    HostedIdentityResult resolved,
    String subject,
    int generation,
    bool canSealBackup, {
    ({String subject, String pubkey})? approvedBackup,
  }) async {
    final saved = await store.forSubject(subject).read();
    if (_disposed || generation != _generation) return false;
    if (account.account?.subject != subject) {
      throw AccountFailure.differentAccount;
    }
    final confirmed =
        saved?.pubkey == resolved.pubkey &&
        (saved?.hasBackupFor(
              backend: HostedEnvironment.origin.origin,
              subject: subject,
            ) ??
            false);
    final needsBackup =
        resolved.state == HostedIdentityState.needsSetup ||
        resolved.state == HostedIdentityState.needsBackup ||
        (resolved.state == HostedIdentityState.needsBinding && !confirmed);
    if (!bindingOnly && needsBackup && !canSealBackup) {
      _set(HostedFlowStage.blocked, HostedFlowProblem.unsupportedPlatform);
      return false;
    }
    final pubkey = resolved.pubkey;
    if ((resolved.state == HostedIdentityState.needsBinding ||
            (!bindingOnly &&
                resolved.state == HostedIdentityState.needsBackup)) &&
        !confirmed &&
        pubkey != null &&
        approvedBackup != (subject: subject, pubkey: pubkey)) {
      _presentBackupConsent(subject, pubkey);
      return false;
    }
    return true;
  }
}
