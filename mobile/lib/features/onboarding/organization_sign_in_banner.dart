import 'dart:async';

import 'package:flutter/material.dart';
import 'package:flutter_hooks/flutter_hooks.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';

import '../../shared/hosted/builderlab_account.dart';
import '../../shared/hosted/hosted_relay_auth.dart';
import '../../shared/hosted/organization_sign_in.dart';
import '../../shared/relay/relay_provider.dart';
import '../../shared/relay/relay_session.dart';
import '../../shared/widgets/buzz_toast.dart';

/// Offers organization sign-in for the active community when it trusts the
/// build's organization. Cached content stays available underneath.
class OrganizationSignInBanner extends HookConsumerWidget {
  const OrganizationSignInBanner({super.key});

  @override
  Widget build(BuildContext context, WidgetRef ref) {
    final signIn = ref.watch(organizationSignInProvider);
    final auth = ref.watch(hostedRelayAuthProvider);
    useListenable(signIn);
    useListenable(auth);
    final config = ref.watch(relayConfigProvider);
    if (signIn == null ||
        !organizationSignInPromptVisible(signIn, auth, config)) {
      return const SizedBox.shrink();
    }
    final relayFailure = organizationRelayFailure(auth, config);
    final String message;
    final String actionLabel;
    final VoidCallback onAction;
    ({String label, VoidCallback onPressed})? secondaryAction;
    switch (signIn.status) {
      case OrganizationSessionStatus.checking:
        return const SizedBox.shrink();
      case OrganizationSessionStatus.signingIn:
        message = 'Finish sign-in in browser';
        actionLabel = 'Cancel';
        onAction = () => unawaited(signIn.cancel());
      case OrganizationSessionStatus.signedOut:
        message = signIn.failure == null
            ? "It looks like you're signed out."
            : 'Sign in failed. Try again.';
        actionLabel = 'Sign in';
        onAction = () => unawaited(signIn.signIn());
      case OrganizationSessionStatus.signedIn:
        final denied = relayFailure == AccountFailure.denied;
        message = denied
            ? 'Denied access to this community.'
            : 'Could not connect.';
        actionLabel = 'Retry';
        onAction = () =>
            unawaited(ref.read(relaySessionProvider.notifier).reconnect());
        if (denied) {
          secondaryAction = (
            label: 'Use a different account',
            onPressed: () => unawaited(signIn.changeAccount()),
          );
        }
    }
    return ReconnectCapsule(
      message: message,
      actionLabel: actionLabel,
      onSignIn: onAction,
      destructiveAction: signIn.status == OrganizationSessionStatus.signingIn,
      secondaryAction: secondaryAction,
    );
  }
}
