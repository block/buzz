import 'package:flutter/material.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';

import '../community/community_provider.dart';
import 'account_identity_store.dart';
import 'organization_sign_in.dart';
import '../widgets/confirmation_dialog.dart';
import '../widgets/feedback_snackbar.dart';

/// Signs an internal build out of its organization: ends the organization
/// session, so the next sign-in goes through single sign-on, then removes the
/// organization's community, which returns to the Sign in landing. It is the
/// only way an internal build signs out.
///
/// The community's key stays on this device as a saved identity, so signing
/// back in reuses it even when the organization holds no backup of it.
Future<void> confirmOrganizationSignOut(
  BuildContext context,
  WidgetRef ref,
) async {
  final messenger = ScaffoldMessenger.of(context);
  final navigator = Navigator.of(context);
  final signIn = ref.read(organizationSignInProvider)!;
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
    // End the session first: a session that could not be cleared would be
    // restored at the next launch, so the community stays until it is.
    await signIn.signOut();
    final failure = signIn.failure;
    if (failure != null) throw failure;
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
