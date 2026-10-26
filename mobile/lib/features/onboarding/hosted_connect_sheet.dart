import 'package:flutter/material.dart';
import 'package:flutter_hooks/flutter_hooks.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';

import '../../shared/hosted/hosted_flow.dart';
import '../../shared/identity_transfer/nsec_import_sheet.dart';
import '../../shared/hosted/hosted_flow_problem_message.dart';
import '../../shared/utils/string_utils.dart';
import '../../shared/theme/grid.dart';
import '../../shared/theme/theme_extensions.dart';
import 'hosted_community_selection.dart';
import '../../shared/hosted/hosted_join_policy_view.dart';
import 'hosted_profile_setup.dart';

/// Presents the app-scoped real flow without retaining credentials or keys.
class HostedConnectSheet extends HookConsumerWidget {
  const HostedConnectSheet({super.key, required this.flow});
  final HostedFlow flow;

  @override
  Widget build(BuildContext context, WidgetRef ref) {
    useListenable(flow);
    final showLinkedIdentity = useState(false);
    Future<void> importIdentity() async {
      Future<void>? pendingImport;
      await showNsecImportSheet(
        context,
        mismatchMessage:
            'This key doesn’t match the identity linked to your Builderlab account.',
        onImport: (nsec) => pendingImport = flow.importIdentity(nsec),
      );
      // Dismissing the sheet does not cancel a secure-store write in flight.
      // Finish that operation before resuming, even when the modal returned null.
      if (pendingImport == null) return;
      try {
        await pendingImport;
      } catch (_) {
        return; // The sheet already presented the import failure.
      }
      if (context.mounted && flow.stage == HostedFlowStage.setupIdentity) {
        await flow.retry();
      }
    }

    final actionStyle = FilledButton.styleFrom(
      minimumSize: const Size.fromHeight(52),
      shape: const StadiumBorder(),
    );
    final working = flow.stage == HostedFlowStage.working;
    final message = switch (flow.stage) {
      HostedFlowStage.idle => 'Continue connecting your account.',
      HostedFlowStage.working =>
        flow.addingCommunityName == null
            ? 'Securing your account and finding communities…'
            : 'Adding ${flow.addingCommunityName}…',
      HostedFlowStage.setupIdentity =>
        flow.canCreateIdentity
            ? 'Create an identity, or bring yours from another device. We’ll back it up for future sign-ins.'
            : 'Your identity hasn’t been backed up yet. Bring it from your other device to keep using the same Buzz account.',
      HostedFlowStage.chooseCommunities => 'Choose your communities',
      HostedFlowStage.communityConflict =>
        '${flow.addingCommunityName} is already saved with a different identity. '
            'Keep that identity and skip this addition, or return to your choices?',
      HostedFlowStage.chooseIdentity =>
        flow.identityChoices.where((choice) => choice.fromCloud).length > 1
            ? 'Choose a backed-up identity to connect to Builderlab. Your other backups and saved community keys will be kept. This app cannot change the selected Builderlab identity later.'
            : flow.identityChoices.any((choice) => choice.fromCloud)
            ? 'Use your Builderlab identity on this device? It differs from your saved community identities. Those keys and your cloud backups will be kept. This app cannot change the selected Builderlab identity later.'
            : 'Choose the identity to connect to Builderlab. It will be bound to your account and backed up. Existing keys will be kept. This app cannot change the selected Builderlab identity later.',
      HostedFlowStage.profile =>
        'What should people in this community call you? You can add a photo in Settings.',
      HostedFlowStage.acceptPolicy =>
        'Review this community’s terms and privacy policy before joining.',
      HostedFlowStage.signIn => 'Complete sign-in in your browser.',
      HostedFlowStage.browserReturned =>
        'Continue setup in your browser. Open your community in Buzz when it is ready.',
      HostedFlowStage.complete => 'Your community is ready.',
      HostedFlowStage.blocked => hostedFlowProblemMessage(flow.problem),
    };
    return Padding(
      padding: const EdgeInsets.fromLTRB(
        Grid.gutter,
        Grid.sm,
        Grid.gutter,
        Grid.lg,
      ),
      child: SingleChildScrollView(
        child: Column(
          mainAxisSize: MainAxisSize.min,
          crossAxisAlignment: CrossAxisAlignment.stretch,
          children: [
            if (flow.stage == HostedFlowStage.setupIdentity) ...[
              Text(
                flow.canCreateIdentity
                    ? 'Make yourself at home'
                    : 'Bring your identity with you',
                style: context.textTheme.titleLarge?.copyWith(
                  fontWeight: FontWeight.w600,
                ),
              ),
              const SizedBox(height: Grid.twelve),
            ],
            Text(message, style: context.textTheme.bodyLarge),
            const SizedBox(height: Grid.sm),
            for (final choice in flow.identityChoices)
              Padding(
                padding: const EdgeInsets.only(bottom: Grid.sm),
                child: OutlinedButton(
                  onPressed: () => flow.chooseIdentity(choice.pubkey),
                  child: Padding(
                    padding: const EdgeInsets.all(Grid.sm),
                    child: Column(
                      children: [
                        Text(choice.label),
                        Text(fullNpub(choice.pubkey) ?? choice.pubkey),
                      ],
                    ),
                  ),
                ),
              ),
            if (flow.stage == HostedFlowStage.setupIdentity) ...[
              if (flow.canCreateIdentity) ...[
                FilledButton(
                  style: actionStyle.copyWith(
                    backgroundColor: WidgetStatePropertyAll(
                      context.colors.primary,
                    ),
                    foregroundColor: WidgetStatePropertyAll(
                      context.colors.onPrimary,
                    ),
                  ),
                  onPressed: flow.createIdentity,
                  child: const Text('Create my Buzz identity'),
                ),
                const SizedBox(height: Grid.xxs),
                TextButton(
                  style: actionStyle,
                  onPressed: importIdentity,
                  child: const Text('Import existing identity'),
                ),
              ] else ...[
                FilledButton(
                  style: actionStyle.copyWith(
                    backgroundColor: WidgetStatePropertyAll(
                      context.colors.primary,
                    ),
                    foregroundColor: WidgetStatePropertyAll(
                      context.colors.onPrimary,
                    ),
                  ),
                  onPressed: importIdentity,
                  child: const Text('Import existing identity'),
                ),
                const SizedBox(height: Grid.twelve),
                Text(
                  'We’ll securely back it up so your next sign-in is easier.',
                  textAlign: TextAlign.center,
                  style: context.textTheme.bodySmall,
                ),
              ],
              if (flow.requiredIdentityPubkey != null) ...[
                const SizedBox(height: Grid.xxs),
                TextButton(
                  onPressed: () =>
                      showLinkedIdentity.value = !showLinkedIdentity.value,
                  child: Text(
                    showLinkedIdentity.value
                        ? 'Hide linked identity'
                        : 'View linked identity',
                  ),
                ),
                if (showLinkedIdentity.value)
                  SelectableText(
                    fullNpub(flow.requiredIdentityPubkey!) ??
                        flow.requiredIdentityPubkey!,
                    style: context.textTheme.bodySmall,
                  ),
              ],
            ],
            if (flow.stage == HostedFlowStage.profile &&
                flow.saveProfile != null)
              HostedProfileSetup(
                save: flow.saveProfile!,
                skip: flow.finishProfile,
              ),
            if (flow.stage == HostedFlowStage.acceptPolicy &&
                flow.joinPolicy != null)
              HostedJoinPolicyView(
                key: ValueKey(flow.joinPolicy!.version),
                policy: flow.joinPolicy!,
                onAccept: flow.acceptJoinPolicy,
              ),
            if (flow.stage == HostedFlowStage.chooseCommunities)
              HostedCommunitySelection(flow: flow),
            if (flow.stage == HostedFlowStage.communityConflict)
              FilledButton(
                onPressed: flow.skipConflictingCommunity,
                child: const Text('Keep saved identity and skip'),
              ),
            if (flow.canChangeCommunities)
              OutlinedButton(
                onPressed: flow.backToCommunityChoices,
                child: const Text('Back to choices'),
              ),
            if ((flow.stage == HostedFlowStage.blocked ||
                    flow.stage == HostedFlowStage.communityConflict) &&
                flow.addedCommunityCount > 0)
              Text(
                '${flow.addedCommunityCount} '
                '${flow.addedCommunityCount == 1 ? "community" : "communities"} added and kept.'
                '${flow.canRetry && flow.stage == HostedFlowStage.blocked ? " Retry continues the remaining additions." : ""}',
              ),
            if (working) const Center(child: CircularProgressIndicator()),
            if (!working &&
                flow.stage != HostedFlowStage.profile &&
                flow.stage != HostedFlowStage.acceptPolicy &&
                flow.stage != HostedFlowStage.chooseIdentity &&
                flow.stage != HostedFlowStage.setupIdentity &&
                flow.stage != HostedFlowStage.chooseCommunities &&
                flow.stage != HostedFlowStage.communityConflict &&
                flow.stage != HostedFlowStage.complete &&
                flow.stage != HostedFlowStage.browserReturned &&
                flow.canRetry)
              FilledButton(
                onPressed: flow.retry,
                child: Text(
                  flow.stage == HostedFlowStage.signIn
                      ? 'Start login again'
                      : 'Try again',
                ),
              ),
          ],
        ),
      ),
    );
  }
}
