import 'dart:async';
import 'package:flutter/material.dart';
import 'package:flutter_hooks/flutter_hooks.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';

import '../../shared/hosted/builderlab_account.dart';
import '../../shared/hosted/hosted_flow.dart';
import '../../shared/hosted/hosted_relay_auth.dart';
import '../../shared/relay/relay_provider.dart';
import '../../shared/relay/relay_session.dart';
import '../../shared/theme/grid.dart';
import '../../shared/theme/app_theme.dart';

/// Leaves cached content and community switching available while disconnected.
class HostedReconnectBanner extends HookConsumerWidget {
  const HostedReconnectBanner({super.key, required this.flow});
  final HostedFlow flow;

  @override
  Widget build(BuildContext context, WidgetRef ref) {
    final auth = ref.watch(hostedRelayAuthProvider);
    useListenable(auth);
    final config = ref.watch(relayConfigProvider);
    if (auth.failedRelay != config.baseUrl ||
        auth.failedPubkey != pubkeyFromNsec(config.nsec) ||
        auth.failure == null) {
      return const SizedBox.shrink();
    }
    final signIn = auth.failure == AccountFailure.signedOut;
    final changeAccount = auth.failure == AccountFailure.denied;
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
              children: [
                Text(
                  signIn
                      ? 'Sign in to Builderlab to reconnect'
                      : auth.failure == AccountFailure.denied
                      ? 'This account cannot reconnect to this community with its saved key.'
                      : 'Could not reconnect to Builderlab.',
                ),
                if (changeAccount)
                  TextButton(
                    onPressed: () => unawaited(
                      ref.read(relaySessionProvider.notifier).reconnect(),
                    ),
                    child: const Text('Retry'),
                  ),
                TextButton(
                  onPressed: () => unawaited(
                    signIn || changeAccount
                        ? flow.reconnectAccount(changeAccount: changeAccount)
                        : ref.read(relaySessionProvider.notifier).reconnect(),
                  ),
                  child: Text(
                    changeAccount
                        ? 'Use a different account'
                        : signIn
                        ? 'Sign in'
                        : 'Retry',
                  ),
                ),
              ],
            ),
          ),
        ),
      ),
    );
  }
}
