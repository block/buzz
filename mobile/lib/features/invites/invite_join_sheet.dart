import 'package:flutter/material.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';
import 'package:lucide_icons_flutter/lucide_icons.dart';

import '../../shared/theme/theme.dart';
import '../pairing/pairing_page.dart';
import 'invite_join_provider.dart';

Future<void> showInviteJoinSheet(BuildContext context, WidgetRef ref) {
  return showModalBottomSheet<void>(
    context: context,
    isScrollControlled: true,
    showDragHandle: true,
    builder: (_) => const InviteJoinSheet(),
  );
}

class InviteJoinSheet extends ConsumerWidget {
  const InviteJoinSheet({super.key});

  @override
  Widget build(BuildContext context, WidgetRef ref) {
    final state = ref.watch(inviteJoinProvider);
    final isClaiming = state.status == InviteJoinStatus.claiming;
    final host = state.host ?? 'unknown host';
    final derivedName = state.communityName;

    if (state.status == InviteJoinStatus.success) {
      return _InviteJoinSuccess(host: host, communityName: derivedName);
    }

    return SafeArea(
      child: Padding(
        padding: const EdgeInsets.fromLTRB(Grid.sm, 0, Grid.sm, Grid.sm),
        child: Column(
          mainAxisSize: MainAxisSize.min,
          crossAxisAlignment: CrossAxisAlignment.stretch,
          children: [
            Icon(LucideIcons.userPlus, size: 40, color: context.colors.primary),
            const SizedBox(height: Grid.sm),
            Text(
              'Join this Buzz community?',
              style: context.textTheme.titleLarge,
            ),
            const SizedBox(height: Grid.xxs),
            Text(
              'Check the relay host before you join:',
              style: context.textTheme.bodyMedium?.copyWith(
                color: context.colors.onSurfaceVariant,
              ),
            ),
            const SizedBox(height: Grid.xs),
            Container(
              padding: const EdgeInsets.all(Grid.twelve),
              decoration: BoxDecoration(
                color: context.colors.surfaceContainerHighest.withValues(
                  alpha: 0.7,
                ),
                borderRadius: BorderRadius.circular(12),
                border: Border.all(color: context.colors.outlineVariant),
              ),
              child: Text(
                host,
                style: context.textTheme.titleMedium?.copyWith(
                  fontFamily: 'GeistMono',
                  fontWeight: FontWeight.w700,
                ),
              ),
            ),
            if (derivedName != null && derivedName != host) ...[
              const SizedBox(height: Grid.xxs),
              Text(
                'Display name: $derivedName',
                style: context.textTheme.bodySmall?.copyWith(
                  color: context.colors.onSurfaceVariant,
                ),
              ),
            ],
            const SizedBox(height: Grid.sm),
            Text(
              'This phone is the only copy of this identity. If you lose it before pairing or backing up, you’ll lose access as this member.',
              style: context.textTheme.bodyMedium?.copyWith(
                color: context.colors.onSurfaceVariant,
              ),
            ),
            if (state.status == InviteJoinStatus.error &&
                state.errorMessage != null) ...[
              const SizedBox(height: Grid.sm),
              Text(
                state.errorMessage!,
                style: context.textTheme.bodySmall?.copyWith(
                  color: context.colors.error,
                ),
              ),
            ],
            const SizedBox(height: Grid.lg),
            Row(
              children: [
                Expanded(
                  child: OutlinedButton(
                    onPressed: isClaiming
                        ? null
                        : () => Navigator.of(context).pop(),
                    child: const Text('Cancel'),
                  ),
                ),
                const SizedBox(width: Grid.sm),
                Expanded(
                  child: FilledButton.icon(
                    onPressed: isClaiming || state.requiresFreshInvite
                        ? null
                        : () => ref
                              .read(inviteJoinProvider.notifier)
                              .confirmJoin(),
                    icon: isClaiming
                        ? const SizedBox(
                            width: 16,
                            height: 16,
                            child: CircularProgressIndicator(strokeWidth: 2),
                          )
                        : const Icon(LucideIcons.check),
                    label: Text(isClaiming ? 'Joining…' : 'Join'),
                  ),
                ),
              ],
            ),
          ],
        ),
      ),
    );
  }
}

class _InviteJoinSuccess extends StatelessWidget {
  final String host;
  final String? communityName;

  const _InviteJoinSuccess({required this.host, this.communityName});

  @override
  Widget build(BuildContext context) {
    return SafeArea(
      child: Padding(
        padding: const EdgeInsets.fromLTRB(Grid.sm, 0, Grid.sm, Grid.sm),
        child: Column(
          mainAxisSize: MainAxisSize.min,
          crossAxisAlignment: CrossAxisAlignment.stretch,
          children: [
            Icon(
              LucideIcons.circleCheck,
              size: 40,
              color: context.colors.primary,
            ),
            const SizedBox(height: Grid.sm),
            Text(
              'You joined ${communityName ?? host}',
              style: context.textTheme.titleLarge,
            ),
            const SizedBox(height: Grid.xs),
            Text(
              'This phone is the only copy of this identity. If you lose it before pairing or backing up, you’ll lose access as this member.',
              style: context.textTheme.bodyMedium?.copyWith(
                color: context.colors.onSurfaceVariant,
              ),
            ),
            const SizedBox(height: Grid.lg),
            FilledButton.icon(
              onPressed: () {
                Navigator.of(context).pop();
                Navigator.of(context).push(
                  MaterialPageRoute<void>(
                    builder: (_) => const PairingPage(addingCommunity: true),
                  ),
                );
              },
              icon: const Icon(LucideIcons.scanLine),
              label: const Text('Back it up now'),
            ),
            const SizedBox(height: Grid.xs),
            TextButton(
              onPressed: () => Navigator.of(context).pop(),
              child: const Text('Not now'),
            ),
          ],
        ),
      ),
    );
  }
}
