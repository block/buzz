import 'package:flutter/material.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';

import '../community/community.dart';
import '../community/community_provider.dart';
import 'account_identity_store.dart';
import 'hosted_environment.dart';
import 'hosted_flow_provider.dart';
import '../widgets/confirmation_dialog.dart';
import '../widgets/feedback_snackbar.dart';

/// How long sign-out waits for removed communities' push leases to be revoked
/// before it ends the Builderlab session those revocations authenticate with.
const _pushLeaseRevocationGrace = Duration(seconds: 10);

/// Signs this device out: removes every community, and in a public build ends
/// the Builderlab session. Each community's key goes to this device's saved
/// identities first, so signing out never destroys a key that may exist
/// nowhere else; signing back in offers it again. In an internal build the
/// Sign in landing that follows ends the organization session.
Future<void> confirmSignOut(BuildContext context, WidgetRef ref) async {
  final messenger = ScaffoldMessenger.of(context);
  final navigator = Navigator.of(context);
  final list = ref.read(communityListProvider.notifier);
  final flow = HostedEnvironment.enabled ? ref.read(hostedFlowProvider) : null;
  final revokePushLeases = ref.read(
    communityPushLeaseRevocationTriggerProvider,
  );
  try {
    final communities = await ref.read(communityListProvider.future);
    final active = await ref.read(activeCommunityProvider.future);
    if (communities.isEmpty || !context.mounted) return;
    final confirmed = await showDestructiveConfirmation(
      context: context,
      title: 'Sign out?',
      message:
          'You’ll need to sign in again to use Buzz on this device. '
          'Your identities stay saved on this device.',
      confirmLabel: 'Sign out',
    );
    if (!confirmed || !context.mounted) return;
    // Every key is saved before any community is removed, so a failure
    // leaves each community with its only copy of its key.
    for (final community in communities) {
      await keepCommunityKey(community);
    }
    // The active community goes last, so removing the others never switches
    // to and connects another one.
    for (final community in [
      ...communities.where((c) => c.id != active?.id),
      ...communities.where((c) => c.id == active?.id),
    ]) {
      await list.removeCommunity(community.id);
    }
    // Reveal the Sign in landing under the page that signed out.
    navigator.popUntil((route) => route.isFirst);
  } catch (e) {
    messenger.showFeedbackSnackBar(
      SnackBar(content: Text('Failed to sign out: $e')),
      kind: ToastKind.error,
    );
    return;
  }
  if (flow == null) return;
  // A hosted relay accepts the tombstones that stop this device's
  // notifications only with the session's assertion. A revocation still
  // outstanding stays journaled and retries on its own schedule.
  try {
    await revokePushLeases().timeout(_pushLeaseRevocationGrace);
  } catch (_) {}
  await flow.signOutAccount();
}

/// Copies [community]'s key into this device's saved identities, where
/// signing in again offers it. A failure throws, so the caller keeps the
/// community rather than delete the only copy of its key.
Future<void> keepCommunityKey(Community community) async {
  final nsec = community.nsec;
  if (nsec == null || nsec.isEmpty) return;
  await AccountIdentityStore()
      .forPairing(AccountIdentityStore.publicKeyForNsec(nsec))
      .create(nsec);
}
