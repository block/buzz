import 'package:flutter/material.dart';
import 'package:gpt_markdown/gpt_markdown.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';
import 'package:lucide_icons_flutter/lucide_icons.dart';

import '../../shared/theme/theme.dart';
import '../../shared/widgets/buzz_loading_indicator.dart';
import '../../shared/widgets/modal_presentation.dart';
import '../pairing/pairing_page.dart';
import 'invite_join_provider.dart';

Future<void> showInviteJoinSheet(BuildContext context, WidgetRef ref) {
  return showBuzzModalBottomSheet<void>(
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
    if (state.status == InviteJoinStatus.reviewingPolicy ||
        state.status == InviteJoinStatus.acceptingPolicy) {
      return _InviteJoinPolicyReview(state: state);
    }
    if (state.status == InviteJoinStatus.declined) {
      return _InviteJoinDeclined(message: state.errorMessage);
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
                        ? SizedBox(
                            width: 16,
                            height: 16,
                            child: BuzzLoadingIndicator(
                              size: 16,
                              semanticLabel: 'Joining community',
                            ),
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

class _InviteJoinPolicyReview extends ConsumerWidget {
  final InviteJoinState state;

  const _InviteJoinPolicyReview({required this.state});

  @override
  Widget build(BuildContext context, WidgetRef ref) {
    final policy = state.policy;
    if (policy == null) return const SizedBox.shrink();

    final isAccepting = state.status == InviteJoinStatus.acceptingPolicy;
    final canAccept =
        (!policy.ageAttestationRequired || state.ageConfirmed) &&
        (!policy.agreementRequired || state.agreementConfirmed);
    final agreementLabel = switch ((
      policy.termsMarkdown != null,
      policy.privacyMarkdown != null,
    )) {
      (true, true) =>
        'I agree to the Terms of Service and Privacy Policy shown above.',
      (true, false) => 'I agree to the Terms of Service shown above.',
      (false, true) => 'I agree to the Privacy Policy shown above.',
      (false, false) => '',
    };
    final notifier = ref.read(inviteJoinProvider.notifier);

    return SafeArea(
      child: SizedBox(
        height: MediaQuery.sizeOf(context).height * 0.82,
        child: Padding(
          padding: const EdgeInsets.fromLTRB(Grid.sm, 0, Grid.sm, Grid.sm),
          child: Column(
            crossAxisAlignment: CrossAxisAlignment.stretch,
            children: [
              Icon(
                LucideIcons.fileCheck,
                size: 40,
                color: context.colors.primary,
              ),
              const SizedBox(height: Grid.sm),
              Text(
                'Review this community’s join policy',
                style: context.textTheme.titleLarge,
              ),
              const SizedBox(height: Grid.xxs),
              Text(
                'Read the policy below, then confirm only the statements that are true for you.',
                style: context.textTheme.bodyMedium?.copyWith(
                  color: context.colors.onSurfaceVariant,
                ),
              ),
              const SizedBox(height: Grid.sm),
              Expanded(
                child: SingleChildScrollView(
                  child: Column(
                    crossAxisAlignment: CrossAxisAlignment.stretch,
                    children: [
                      if (policy.termsMarkdown case final terms?) ...[
                        Text(
                          'Terms of Service',
                          style: context.textTheme.titleMedium,
                        ),
                        const SizedBox(height: Grid.xxs),
                        GptMarkdown(terms, style: context.textTheme.bodyMedium),
                        const SizedBox(height: Grid.sm),
                      ],
                      if (policy.privacyMarkdown case final privacy?) ...[
                        Text(
                          'Privacy Policy',
                          style: context.textTheme.titleMedium,
                        ),
                        const SizedBox(height: Grid.xxs),
                        GptMarkdown(
                          privacy,
                          style: context.textTheme.bodyMedium,
                        ),
                        const SizedBox(height: Grid.sm),
                      ],
                      if (policy.ageAttestationRequired)
                        _PolicyCheckbox(
                          value: state.ageConfirmed,
                          onChanged: isAccepting
                              ? null
                              : notifier.setAgeConfirmed,
                          label: 'I am 18 years of age or older.',
                        ),
                      if (policy.agreementRequired) ...[
                        if (policy.ageAttestationRequired)
                          const SizedBox(height: Grid.xxs),
                        _PolicyCheckbox(
                          value: state.agreementConfirmed,
                          onChanged: isAccepting
                              ? null
                              : notifier.setAgreementConfirmed,
                          label: agreementLabel,
                        ),
                      ],
                      if (state.errorMessage != null) ...[
                        const SizedBox(height: Grid.sm),
                        Text(
                          state.errorMessage!,
                          style: context.textTheme.bodySmall?.copyWith(
                            color: context.colors.error,
                          ),
                        ),
                      ],
                    ],
                  ),
                ),
              ),
              const SizedBox(height: Grid.sm),
              Row(
                children: [
                  Expanded(
                    child: OutlinedButton(
                      onPressed: isAccepting ? null : notifier.declinePolicy,
                      child: const Text('Decline'),
                    ),
                  ),
                  const SizedBox(width: Grid.sm),
                  Expanded(
                    child: FilledButton.icon(
                      key: const Key('accept-join-policy'),
                      onPressed: isAccepting || !canAccept
                          ? null
                          : notifier.acceptPolicy,
                      icon: isAccepting
                          ? SizedBox(
                              width: 16,
                              height: 16,
                              child: BuzzLoadingIndicator(
                                size: 16,
                                semanticLabel: 'Accepting join policy',
                              ),
                            )
                          : const Icon(LucideIcons.check),
                      label: Text(
                        isAccepting ? 'Accepting…' : 'Accept and join',
                      ),
                    ),
                  ),
                ],
              ),
            ],
          ),
        ),
      ),
    );
  }
}

class _PolicyCheckbox extends StatelessWidget {
  final bool value;
  final ValueChanged<bool>? onChanged;
  final String label;

  const _PolicyCheckbox({
    required this.value,
    required this.onChanged,
    required this.label,
  });

  @override
  Widget build(BuildContext context) {
    return InkWell(
      onTap: onChanged == null ? null : () => onChanged!(!value),
      borderRadius: BorderRadius.circular(12),
      child: Padding(
        padding: const EdgeInsets.symmetric(vertical: Grid.half),
        child: Row(
          crossAxisAlignment: CrossAxisAlignment.start,
          children: [
            Checkbox(
              value: value,
              onChanged: onChanged == null
                  ? null
                  : (checked) => onChanged!(checked ?? false),
            ),
            const SizedBox(width: Grid.xxs),
            Expanded(
              child: Padding(
                padding: const EdgeInsets.only(top: Grid.twelve),
                child: Text(label, style: context.textTheme.bodyMedium),
              ),
            ),
          ],
        ),
      ),
    );
  }
}

class _InviteJoinDeclined extends StatelessWidget {
  final String? message;

  const _InviteJoinDeclined({this.message});

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
              LucideIcons.circleX,
              size: 40,
              color: context.colors.onSurfaceVariant,
            ),
            const SizedBox(height: Grid.sm),
            Text('Community not joined', style: context.textTheme.titleLarge),
            const SizedBox(height: Grid.xxs),
            Text(
              message ?? 'You did not accept this community’s join policy.',
              style: context.textTheme.bodyMedium?.copyWith(
                color: context.colors.onSurfaceVariant,
              ),
            ),
            const SizedBox(height: Grid.lg),
            FilledButton(
              onPressed: () => Navigator.of(context).pop(),
              child: const Text('Close'),
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
