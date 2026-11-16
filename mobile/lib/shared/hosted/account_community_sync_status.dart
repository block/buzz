import 'package:flutter/material.dart';
import 'package:flutter_hooks/flutter_hooks.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';

import '../community/community_provider.dart';
import '../widgets/confirmation_dialog.dart';
import '../widgets/feedback_snackbar.dart';
import 'builderlab_account.dart';
import 'hosted_flow_provider.dart';
import 'hosted_relay_auth.dart';
import 'known_community_sync_provider.dart';
import 'known_community_address.dart';

/// Durable removals stay visible until the account service acknowledges them.
final pendingAccountRemovalsProvider =
    FutureProvider.autoDispose<({String? subject, Set<String> urls})>((
      ref,
    ) async {
      ref.watch(communityListProvider);
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
      if (subject == null || sync == null) {
        return (subject: subject, urls: <String>{});
      }
      return (subject: subject, urls: await sync.pendingRemovals(subject));
    });

/// Shows queued/failed account edits separately from local community access.
class AccountCommunitySyncStatus extends HookConsumerWidget {
  /// Shows status only for the currently selected account.
  const AccountCommunitySyncStatus({super.key});

  @override
  Widget build(BuildContext context, WidgetRef ref) {
    final sync = ref.watch(knownCommunitySyncProvider);
    useListenable(sync);
    final account = ref.watch(builderlabAccountProvider);
    useListenable(account.subjectChanges);
    final pending = ref.watch(pendingAccountRemovalsProvider);
    final value = pending.value;
    final removals = value?.subject == account.account?.subject
        ? value?.urls ?? <String>{}
        : <String>{};
    final subject = account.account?.subject;
    final owner = subject == null
        ? null
        : '${account.accountOrigin.origin}\n$subject';
    final local = (ref.watch(communityListProvider).value ?? const [])
        .where((c) => owner != null && c.accountSyncOwner == owner)
        .toList();
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
          for (final url in removals)
            if (local.any((c) => knownCommunityAddress(c.relayUrl) == url))
              TextButton(
                onPressed: () async {
                  if (subject == null || account.account?.subject != subject) {
                    return;
                  }
                  final confirmed = await showDestructiveConfirmation(
                    context: context,
                    title: 'Finish removal?',
                    message:
                        'Remove the remaining identities for ${Uri.parse(url).authority} from this device and sync account removal?',
                    confirmLabel: 'Remove',
                  );
                  if (!confirmed ||
                      !context.mounted ||
                      account.account?.subject != subject) {
                    return;
                  }
                  try {
                    for (final item in local.where(
                      (c) => knownCommunityAddress(c.relayUrl) == url,
                    )) {
                      await ref
                          .read(communityListProvider.notifier)
                          .removeCommunity(
                            item.id,
                            confirmedSubject: subject,
                            accountConfirmed: true,
                          );
                    }
                    sync?.retry();
                  } catch (_) {
                    if (context.mounted) {
                      ScaffoldMessenger.of(context).showFeedbackSnackBar(
                        const SnackBar(
                          content: Text(
                            'Removal could not finish. Please try again.',
                          ),
                        ),
                        kind: ToastKind.error,
                      );
                    }
                  }
                },
                child: Text(
                  'Finish removing ${Uri.parse(url).authority} from this device',
                ),
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
          if (sync?.failure == AccountFailure.accountSetupRequired)
            TextButton(
              onPressed: () => ref.read(hostedFlowProvider).signInAccount(),
              child: const Text('Complete account setup'),
            ),
          if (sync?.failure == AccountFailure.denied)
            TextButton(
              onPressed: () => ref
                  .read(hostedFlowProvider)
                  .signInAccount(reauthenticate: true),
              child: const Text('Sign in again'),
            ),
        ],
      ),
    );
  }
}
