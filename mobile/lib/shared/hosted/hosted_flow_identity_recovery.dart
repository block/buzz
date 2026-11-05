part of 'hosted_flow.dart';

const _verifyPath = '/v1/buzz/nostr-identities/verify';

/// Definitive rejection of binding an account's chosen key, valid only for
/// the attempt that observed it.
typedef _RejectedBinding = ({String subject, String pubkey, int generation});

/// Recovery when binding the chosen key was definitively rejected.
extension HostedFlowIdentityRecovery on HostedFlow {
  bool get canChooseDifferentIdentity {
    final rejected = _rejectedBinding;
    // A paired transfer pins its key, so there is no other identity to choose.
    return _pairedPubkey == null &&
        stage == HostedFlowStage.blocked &&
        problem == HostedFlowProblem.requestRejected &&
        rejected != null &&
        rejected.generation == _generation &&
        rejected.subject == account.account?.subject;
  }

  /// Release this account's copy of the rejected key (kept elsewhere) and
  /// return to identity choice.
  Future<void> chooseDifferentIdentity() {
    final rejected = _rejectedBinding;
    if (!canChooseDifferentIdentity || rejected == null) return Future.value();
    final generation = _nextGeneration();
    _set(HostedFlowStage.working);
    return _enqueue(() async {
      var released = false;
      try {
        released = await identity.releaseRejected(
          rejected.pubkey,
          subject: rejected.subject,
          isCurrent: _isCurrent(generation),
        );
      } catch (_) {
        if (_disposed ||
            generation != _generation ||
            account.account?.subject != rejected.subject) {
          rethrow;
        }
      }
      if (_disposed || generation != _generation) return;
      if (released) return _continue(generation);
      // The rejection still stands, so the choice can be made again.
      _rejectedBinding = (
        subject: rejected.subject,
        pubkey: rejected.pubkey,
        generation: generation,
      );
      _set(HostedFlowStage.blocked, HostedFlowProblem.requestRejected);
    });
  }

  /// Run [bind], recording only a definitive 4xx on verify for this attempt,
  /// account and key as evidence against it.
  Future<void> _observeBinding(
    String subject,
    String pubkey,
    int generation,
    Future<void> Function() bind,
  ) async {
    account.lastRejectedRequest = null;
    try {
      await bind();
    } on AccountFailure catch (failure) {
      if (failure == AccountFailure.rejected &&
          account.lastRejectedRequest?.path == _verifyPath &&
          account.account?.subject == subject) {
        _rejectedBinding = (
          subject: subject,
          pubkey: pubkey,
          generation: generation,
        );
      }
      rethrow;
    }
  }
}
