part of 'hosted_flow.dart';

enum HostedFlowStage {
  idle,
  working,
  signIn,
  chooseIdentity,
  setupIdentity,
  chooseCommunities,
  communityConflict,
  acceptPolicy,
  profile,
  blocked,
  browserReturned,
  complete,
}

enum HostedFlowProblem {
  retry,
  denied,
  accountSetupRequired,
  differentAccount,
  identityUnreadable,
  backupUnavailable,
  unsupportedPlatform,
  invalidResponse,
  requestRejected,
  identityMismatch,
  identityUnavailable,
  backupRequired,
  bindingRequired,
  unsupportedEnvironment,
  invitePolicyRequired,
  inviteExpired,
  inviteExhausted,
  inviteInvalid,
  inviteRestricted,
  inviteOwnerMismatch,
  communityCreationRejected,
}

/// Resolves only after relay admission is proven and local persistence completes.
/// Implementations must check [isCurrent] immediately before durable mutations.
typedef HostedCommunityAdmission =
    Future<void> Function(
      BuzzDeepLink target,
      String subject,
      String pubkey,
      bool Function() isCurrent,
    );

/// Reopens the completed Add destination after restoring older memberships.
typedef HostedCommunityReturn =
    Future<void> Function(bool Function() isCurrent);

/// Account-scoped picker state retained while the Add route is open.
class HostedCommunityChoices {
  HostedCommunityChoices._(
    this._subject,
    this._communities,
    this._selectedRelays,
    this._landingRelay,
    this._addedRelays,
  );

  final String _subject;
  final List<AddCommunityDeepLink> _communities;
  final Set<String> _selectedRelays;
  final String? _landingRelay;
  final Set<String> _addedRelays;
}
