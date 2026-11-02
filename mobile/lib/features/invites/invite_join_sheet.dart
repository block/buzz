import 'package:flutter/material.dart';
import 'package:flutter/foundation.dart';
import '../../shared/identity_transfer/native_community_entry.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';
import 'package:buzz/shared/theme/buzz_icons.dart';

import '../../shared/theme/theme.dart';
import '../../shared/widgets/buzz_loading_indicator.dart';
import '../../shared/widgets/modal_presentation.dart';
import '../pairing/pairing_page.dart';
import 'invite_join_provider.dart';

Future<bool?> showInviteJoinSheet(BuildContext context) async {
  final container = ProviderScope.containerOf(context);
  final state = container.read(inviteJoinProvider);
  if (defaultTargetPlatform == TargetPlatform.iOS &&
      state.status == InviteJoinStatus.confirming &&
      !state.isStarterSetupRecovery) {
    final joined = await NativeCommunityEntry.show(
      name: state.host ?? 'Unknown community',
      dark: Theme.of(context).brightness == Brightness.dark,
      primaryArgb: Theme.of(context).colorScheme.primary.toARGB32(),
      choices: const [],
      connect: (_, _, _, _) async {},
      confirmInvite: () async {
        final before = container.read(inviteJoinProvider);
        if (before.requiresFreshInvite) {
          return {
            'error': before.errorMessage ?? 'Open a fresh invite to continue.',
          };
        }
        await container.read(inviteJoinProvider.notifier).confirmJoin();
        final result = container.read(inviteJoinProvider);
        return result.status == InviteJoinStatus.success
            ? {'joined': true}
            : {'error': result.errorMessage ?? 'Could not join. Try again.'};
      },
    );
    if (joined == false) return false;
    if (!context.mounted) return joined;
    // Preserve the existing success screen and starter-channel destination.
  }
  return showBuzzModalBottomSheet<bool>(
    context: context,
    isScrollControlled: true,
    // The route remains a normal modal while idle, but its contents install a
    // PopScope once a membership claim or starter setup begins. Keep drag and
    // the shared close affordance out of this sheet so they cannot bypass that
    // in-flight guard.
    enableDrag: false,
    showCloseButton: false,
    builder: (_) => const _InviteJoinSheetRoute(),
  );
}

class _InviteJoinSheetRoute extends ConsumerWidget {
  const _InviteJoinSheetRoute();

  @override
  Widget build(BuildContext context, WidgetRef ref) {
    final isClaiming =
        ref.watch(inviteJoinProvider).status == InviteJoinStatus.claiming;
    return PopScope(canPop: !isClaiming, child: const InviteJoinSheet());
  }
}

class InviteJoinSheet extends ConsumerWidget {
  const InviteJoinSheet({super.key});

  @override
  Widget build(BuildContext context, WidgetRef ref) {
    final state = ref.watch(inviteJoinProvider);
    final isClaiming = state.status == InviteJoinStatus.claiming;
    final isStarterSetupRecovery = state.isStarterSetupRecovery;
    final host = state.host ?? 'unknown host';
    final derivedName = state.communityName;
    final primaryLabel = switch ((
      isClaiming,
      isStarterSetupRecovery,
      state.status,
    )) {
      (true, true, _) => 'Finishing setup…',
      (true, false, _) => 'Joining…',
      (false, true, InviteJoinStatus.error) => 'Retry setup',
      (false, true, _) => 'Finish setting up',
      _ => 'Join',
    };

    if (state.status == InviteJoinStatus.success) {
      return _InviteJoinSuccess(
        host: host,
        communityName: derivedName,
        hasFocusChannel: state.focusChannelId != null,
      );
    }

    return SafeArea(
      child: SingleChildScrollView(
        padding: const EdgeInsets.fromLTRB(Grid.sm, 0, Grid.sm, Grid.sm),
        child: Column(
          mainAxisSize: MainAxisSize.min,
          crossAxisAlignment: CrossAxisAlignment.stretch,
          children: [
            Icon(BuzzIcons.userPlus, size: 40, color: context.colors.primary),
            const SizedBox(height: Grid.sm),
            Text(
              isStarterSetupRecovery
                  ? 'Finish setting up'
                  : 'Join this Buzz community?',
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
                borderRadius: BorderRadius.circular(Radii.container),
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
            if (isStarterSetupRecovery) ...[
              Text(
                'Your membership is ready, but starter channels still need to be set up. You can safely retry without claiming the invite again.',
                style: context.textTheme.bodyMedium?.copyWith(
                  color: context.colors.onSurfaceVariant,
                ),
              ),
              const SizedBox(height: Grid.sm),
            ],
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
                        : () async => ref
                              .read(inviteJoinProvider.notifier)
                              .confirmJoin(),
                    icon: isClaiming
                        ? SizedBox(
                            width: 16,
                            height: 16,
                            child: BuzzLoadingIndicator(
                              size: 16,
                              semanticLabel: isStarterSetupRecovery
                                  ? 'Finishing setup'
                                  : 'Joining community',
                            ),
                          )
                        : const Icon(BuzzIcons.check),
                    label: Text(primaryLabel),
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
  final bool hasFocusChannel;

  const _InviteJoinSuccess({
    required this.host,
    required this.hasFocusChannel,
    this.communityName,
  });

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
              BuzzIcons.circleCheck,
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
              icon: const Icon(BuzzIcons.scanLine),
              label: const Text('Back it up now'),
            ),
            const SizedBox(height: Grid.xs),
            TextButton(
              onPressed: () => Navigator.of(context).pop(hasFocusChannel),
              child: Text(
                hasFocusChannel ? 'Continue to #welcome-everyone' : 'Not now',
              ),
            ),
          ],
        ),
      ),
    );
  }
}
