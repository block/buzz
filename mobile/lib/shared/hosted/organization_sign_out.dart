import 'package:flutter/material.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';

import '../community/community_provider.dart';
import '../widgets/confirmation_dialog.dart';
import '../widgets/feedback_snackbar.dart';

/// Signs an internal build out of its organization. Removing the
/// organization's community returns to the Sign in landing, which ends the
/// organization session; it is the only way an internal build signs out.
Future<void> confirmOrganizationSignOut(
  BuildContext context,
  WidgetRef ref,
) async {
  final messenger = ScaffoldMessenger.of(context);
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
    await ref
        .read(communityListProvider.notifier)
        .removeCommunity(community.id);
  } catch (e) {
    messenger.showFeedbackSnackBar(
      SnackBar(content: Text('Failed to sign out: $e')),
      kind: ToastKind.error,
    );
  }
}
