import 'package:flutter/material.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';

import '../community/community_provider.dart';
import 'account_identity_store.dart';
import '../widgets/confirmation_dialog.dart';
import '../widgets/feedback_snackbar.dart';

/// Signs an internal build out of its organization. Removing the
/// organization's community returns to the Sign in landing, which ends the
/// organization session; it is the only way an internal build signs out.
///
/// The community's key stays on this device as a saved identity, so signing
/// back in reuses it even when the organization holds no backup of it.
Future<void> confirmOrganizationSignOut(
  BuildContext context,
  WidgetRef ref,
) async {
  final messenger = ScaffoldMessenger.of(context);
  final navigator = Navigator.of(context);
  try {
    final community = await ref.read(activeCommunityProvider.future);
    if (community == null || !context.mounted) return;
    final confirmed = await showDestructiveConfirmation(
      context: context,
      title: 'Sign out?',
      message: 'You’ll need to sign in again to use Buzz on this device.',
      confirmLabel: 'Sign out',
    );
    if (!confirmed || !context.mounted) return;
    final nsec = community.nsec;
    if (nsec != null && nsec.isNotEmpty) {
      // A failure here keeps the community rather than delete the only copy
      // of its key.
      await AccountIdentityStore()
          .forPairing(AccountIdentityStore.publicKeyForNsec(nsec))
          .create(nsec);
    }
    await ref
        .read(communityListProvider.notifier)
        .removeCommunity(community.id);
    // Reveal the Sign in landing under the page that signed out.
    navigator.popUntil((route) => route.isFirst);
  } catch (e) {
    messenger.showFeedbackSnackBar(
      SnackBar(content: Text('Failed to sign out: $e')),
      kind: ToastKind.error,
    );
  }
}
