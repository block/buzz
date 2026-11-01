import 'package:flutter/material.dart';
import 'package:flutter/cupertino.dart';
import 'package:flutter_hooks/flutter_hooks.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';

import '../../shared/hosted/hosted_flow.dart';
import '../../shared/widgets/onboarding_step_page.dart';
import '../../shared/widgets/bee_loading_mark.dart';
import '../../shared/widgets/onboarding_loading_page.dart';
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
  const HostedConnectSheet({
    super.key,
    required this.flow,
    this.onAddCommunity,
    this.fullScreen = false,
    this.onBack,
    this.presentedStage,
  });
  final HostedFlow flow;
  final VoidCallback? onAddCommunity;
  final bool fullScreen;
  final VoidCallback? onBack;
  final HostedFlowStage? presentedStage;

  @override
  Widget build(BuildContext context, WidgetRef ref) {
    useListenable(flow);
    final stage = presentedStage ?? flow.stage;
    final showLinkedIdentity = useState(false);
    Future<void> importIdentity() async {
      Future<void>? pendingImport;
      Future<void> save(String nsec) =>
          pendingImport = flow.importIdentity(nsec);
      const mismatch =
          'This key doesn’t match the identity linked to your Builderlab account.';
      if (fullScreen) {
        await Navigator.of(context).push<bool>(
          CupertinoPageRoute(
            builder: (_) => NsecImportSheet(
              onImport: save,
              mismatchMessage: mismatch,
              fullScreen: true,
            ),
          ),
        );
      } else {
        await showNsecImportSheet(
          context,
          mismatchMessage: mismatch,
          onImport: save,
        );
      }
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
    final working = stage == HostedFlowStage.working;
    final message = switch (stage) {
      HostedFlowStage.idle => 'Continue connecting your account.',
      HostedFlowStage.working =>
        flow.addingCommunityName == null
            ? 'Securing your account and finding communities…'
            : 'Adding ${flow.addingCommunityName}…',
      HostedFlowStage.setupIdentity =>
        flow.canCreateIdentity
            ? 'Your Buzz identity is how communities recognize you. Create a new one, or import your existing identity if you already use Buzz.'
            : 'Your identity hasn’t been backed up yet. Bring it from your other device to keep using the same Buzz account.',
      HostedFlowStage.chooseCommunities => 'Select a community',
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
    if (fullScreen && stage == HostedFlowStage.chooseCommunities) {
      return HostedCommunitySelection(
        flow: flow,
        fullScreen: true,
        onAddCommunity: onAddCommunity,
        onBack: onBack,
      );
    }
    if (fullScreen &&
        stage == HostedFlowStage.acceptPolicy &&
        flow.joinPolicy != null) {
      return HostedJoinPolicyView(
        key: ValueKey(flow.joinPolicy!.version),
        policy: flow.joinPolicy!,
        onAccept: flow.acceptJoinPolicy,
        fullScreen: true,
        onBack: onBack,
      );
    }
    final children = <Widget>[
      if (!fullScreen && stage == HostedFlowStage.setupIdentity) ...[
        Text(
          flow.canCreateIdentity
              ? 'Create or import a Buzz identity'
              : 'Bring your identity with you',
          style: context.textTheme.titleLarge?.copyWith(
            fontWeight: FontWeight.w600,
          ),
        ),
        const SizedBox(height: Grid.twelve),
      ],
      if (!fullScreen) Text(message, style: context.textTheme.bodyLarge),
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
      if (stage == HostedFlowStage.setupIdentity) ...[
        if (flow.canCreateIdentity) ...[
          FilledButton(
            style: actionStyle.copyWith(
              backgroundColor: WidgetStatePropertyAll(context.colors.primary),
              foregroundColor: WidgetStatePropertyAll(context.colors.onPrimary),
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
              backgroundColor: WidgetStatePropertyAll(context.colors.primary),
              foregroundColor: WidgetStatePropertyAll(context.colors.onPrimary),
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
      if (stage == HostedFlowStage.profile && flow.saveProfile != null)
        HostedProfileSetup(save: flow.saveProfile!, skip: flow.finishProfile),
      if (stage == HostedFlowStage.acceptPolicy && flow.joinPolicy != null)
        HostedJoinPolicyView(
          key: ValueKey(flow.joinPolicy!.version),
          policy: flow.joinPolicy!,
          onAccept: flow.acceptJoinPolicy,
        ),
      if (stage == HostedFlowStage.chooseCommunities)
        HostedCommunitySelection(flow: flow, onAddCommunity: onAddCommunity),
      if (stage == HostedFlowStage.communityConflict)
        FilledButton(
          onPressed: flow.skipConflictingCommunity,
          child: const Text('Keep saved identity and skip'),
        ),
      if (flow.canSkipFailedCommunity)
        TextButton(
          onPressed: flow.skipFailedCommunity,
          child: const Text('Skip this community'),
        ),
      if (flow.canChangeCommunities)
        TextButton(
          onPressed: flow.backToCommunityChoices,
          child: const Text('Back to choices'),
        ),
      if ((stage == HostedFlowStage.blocked ||
              stage == HostedFlowStage.communityConflict) &&
          flow.addedCommunityCount > 0)
        Text(
          '${flow.addedCommunityCount} '
          '${flow.addedCommunityCount == 1 ? "community" : "communities"} added and kept.'
          '${flow.canRetry && stage == HostedFlowStage.blocked ? " Retry continues the remaining additions." : ""}',
        ),
      if (working && !fullScreen) const Center(child: BeeLoadingMark()),
      if (!working &&
          stage != HostedFlowStage.profile &&
          stage != HostedFlowStage.acceptPolicy &&
          stage != HostedFlowStage.chooseIdentity &&
          stage != HostedFlowStage.setupIdentity &&
          stage != HostedFlowStage.chooseCommunities &&
          stage != HostedFlowStage.communityConflict &&
          stage != HostedFlowStage.complete &&
          stage != HostedFlowStage.browserReturned &&
          flow.canRetry)
        FilledButton(
          onPressed: flow.retry,
          child: Text(
            stage == HostedFlowStage.signIn ? 'Start login again' : 'Try again',
          ),
        ),
    ];
    if (!fullScreen) {
      return SingleChildScrollView(
        padding: const EdgeInsets.all(Grid.gutter),
        child: Column(
          crossAxisAlignment: CrossAxisAlignment.stretch,
          mainAxisSize: MainAxisSize.min,
          children: children,
        ),
      );
    }
    final title = switch (stage) {
      HostedFlowStage.setupIdentity =>
        flow.canCreateIdentity
            ? 'Create or import a Buzz identity'
            : 'Bring your identity with you',
      HostedFlowStage.chooseIdentity => 'Choose your identity',
      HostedFlowStage.working =>
        flow.addingCommunityName == null
            ? 'Getting ready…'
            : 'Joining your community',
      HostedFlowStage.blocked => 'Let’s try that again',
      HostedFlowStage.communityConflict => 'Keep your saved identity',
      HostedFlowStage.browserReturned => 'Continue in your browser',
      _ => 'Connect to Builderlab',
    };
    if (working) return OnboardingLoadingPage(title: title, onCancel: onBack);
    bool isAction(Widget widget) => widget is ButtonStyleButton;
    return OnboardingStepPage(
      title: title,
      subtitle: message,
      onBack: onBack ?? () {},
      content: Column(
        crossAxisAlignment: CrossAxisAlignment.stretch,
        children: children.where((widget) => !isAction(widget)).toList(),
      ),
      actions: children.where(isAction).toList(),
    );
  }
}
