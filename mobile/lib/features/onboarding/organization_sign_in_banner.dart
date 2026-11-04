import 'dart:async';

import 'package:flutter/material.dart';
import 'package:flutter_hooks/flutter_hooks.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';

import '../../shared/hosted/builderlab_account.dart';
import '../../shared/hosted/hosted_relay_auth.dart';
import '../../shared/hosted/organization_sign_in.dart';
import '../../shared/relay/relay_provider.dart';
import '../../shared/relay/relay_session.dart';
import '../../shared/theme/app_theme.dart';
import '../../shared/theme/grid.dart';

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
    final List<Widget> actions;
    switch (signIn.status) {
      case OrganizationSessionStatus.checking:
        // Unreachable: organizationSignInPromptVisible hides checking.
        return const SizedBox.shrink();
      case OrganizationSessionStatus.signingIn:
        message = 'Finishing organization sign-in…';
        actions = [
          TextButton(
            onPressed: () => unawaited(signIn.cancel()),
            child: const Text('Cancel sign-in'),
          ),
        ];
      case OrganizationSessionStatus.signedOut:
        message = switch (signIn.failure) {
          null =>
            'Sign in with your organization to stay connected to this community.',
          AccountFailure.storage =>
            'Your saved organization sign-in could not be read. Sign in again to replace it.',
          _ => 'Organization sign-in did not finish. Try again.',
        };
        actions = [
          TextButton(
            onPressed: () => unawaited(signIn.signIn()),
            child: const Text('Sign in'),
          ),
        ];
      case OrganizationSessionStatus.signedIn:
        // The check above hides signed out and unavailable, which reconnects.
        message = relayFailure == AccountFailure.denied
            ? 'Your organization account cannot use this community with this key.'
            : 'Could not reconnect with your organization account.';
        actions = [
          TextButton(
            onPressed: () =>
                unawaited(ref.read(relaySessionProvider.notifier).reconnect()),
            child: const Text('Retry'),
          ),
          // Retrying the same refused account fails the same way.
          if (relayFailure == AccountFailure.denied)
            TextButton(
              onPressed: () => unawaited(signIn.changeAccount()),
              child: const Text('Use a different account'),
            ),
        ];
    }
    return Positioned(
      left: Grid.sm,
      right: Grid.sm,
      bottom: 0,
      child: SafeArea(
        child: Material(
          elevation: 4,
          borderRadius: BorderRadius.circular(Radii.lg),
          child: Padding(
            padding: const EdgeInsets.all(Grid.sm),
            child: Column(
              mainAxisSize: MainAxisSize.min,
              children: [Text(message), ...actions],
            ),
          ),
        ),
      ),
    );
  }
}
