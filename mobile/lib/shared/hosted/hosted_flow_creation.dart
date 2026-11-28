part of 'hosted_flow.dart';

/// Provisioning for [HostedCreateCommunity] targets.
extension _HostedFlowCreation on HostedFlow {
  /// The community to enter, or null when the address is taken. Nothing was
  /// created then, so setup ends and [HostedFlowProblem.communityNameTaken]
  /// sends the user back to the create form to choose another address.
  Future<BuzzDeepLink?> _destinationFor(
    BuzzDeepLink target,
    String pubkey,
    int generation,
  ) async {
    if (target is! HostedCreateCommunity) return target;
    try {
      return await target.provision(account, pubkey, _isCurrent(generation));
    } on HostedCreationFailure catch (failure) {
      if (failure != HostedCreationFailure.taken) rethrow;
      final leaving = leave();
      final left = _generation;
      await leaving;
      if (!_disposed && left == _generation && stage == HostedFlowStage.idle) {
        _set(HostedFlowStage.idle, HostedFlowProblem.communityNameTaken);
      }
      return null;
    }
  }
}
