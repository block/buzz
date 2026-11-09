import 'package:flutter/material.dart';
import 'package:flutter_hooks/flutter_hooks.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';

import 'builderlab_account.dart';
import 'hosted_flow_provider.dart';
import 'hosted_relay_auth.dart';
import 'known_community_sync_provider.dart';

/// Durable removals stay visible until the account service acknowledges them.
final pendingAccountRemovalsProvider = FutureProvider.autoDispose<Set<String>>((
  ref,
) async {
  final sync = ref.watch(knownCommunitySyncProvider);
  final account = ref.watch(builderlabAccountProvider);
  void changed() => ref.invalidateSelf();
  sync?.addListener(changed);
  account.subjectChanges.addListener(changed);
  ref.onDispose(() {
    sync?.removeListener(changed);
    account.subjectChanges.removeListener(changed);
  });
  final subject = account.account?.subject;
  if (subject == null || sync == null) return {};
  return sync.pendingRemovals(subject);
});

/// Shows queued/failed account edits separately from local community access.
class AccountCommunitySyncStatus extends HookConsumerWidget {
  const AccountCommunitySyncStatus({super.key});

  @override
  Widget build(BuildContext context, WidgetRef ref) {
    final sync = ref.watch(knownCommunitySyncProvider);
    useListenable(sync);
    final pending = ref.watch(pendingAccountRemovalsProvider);
    final removals = pending.value ?? <String>{};
    final failed = sync?.failure != null || pending.hasError;
    if (removals.isEmpty && !failed) return const SizedBox.shrink();
    return Semantics(
      liveRegion: true,
      child: Column(
        children: [
          for (final url in removals)
            Text(
              failed
                  ? 'Removal of ${Uri.parse(url).authority} hasn’t synced to your account.'
                  : 'Removal of ${Uri.parse(url).authority} is pending account sync.',
            ),
          if (failed && removals.isEmpty)
            const Text('Could not sync your account communities.'),
          TextButton(
            onPressed: () {
              sync?.retry();
              ref.invalidate(pendingAccountRemovalsProvider);
            },
            child: const Text('Retry account sync'),
          ),
          if (sync?.failure == AccountFailure.accountSetupRequired ||
              sync?.failure == AccountFailure.denied)
            TextButton(
              onPressed: () => ref.read(hostedFlowProvider).signInAccount(),
              child: const Text('Complete account setup'),
            ),
        ],
      ),
    );
  }
}
