import 'dart:async';

import 'package:flutter/material.dart';
import 'package:flutter_hooks/flutter_hooks.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';

import 'features/pairing/pairing_page.dart';
import 'features/pairing/pairing_provider.dart';
import 'shared/hosted/hosted_flow_provider.dart';
import 'shared/hosted/hosted_flow.dart';
import 'shared/hosted/hosted_communities.dart';
import 'shared/hosted/use_known_communities.dart';
import 'shared/hosted/hosted_environment.dart';
import 'shared/hosted/hosted_community_creation.dart';
import 'shared/hosted/hosted_flow_problem_message.dart';
import 'shared/widgets/confirmation_dialog.dart';

/// Navigation over the existing branded welcome surface.
class OnboardingAppEntry extends HookConsumerWidget {
  const OnboardingAppEntry({
    super.key,
    this.addingCommunity = false,
    this.restoreAccount = true,
    this.onExit,
    this.onComplete,
  });
  final bool addingCommunity;
  final bool restoreAccount;
  final VoidCallback? onExit;

  /// Continues picker restoration before the completed Add route is removed.
  final VoidCallback? onComplete;

  @override
  Widget build(BuildContext context, WidgetRef ref) {
    final existingUser = useState(false);
    final creating = useState(false);
    final enteringCommunityLink = useState(false);
    final flow = HostedEnvironment.enabled
        ? ref.watch(hostedFlowProvider)
        : null;
    useListenable(flow);
    useListenable(flow?.account.subjectChanges);
    useEffect(() {
      if (flow == null || !restoreAccount) return null;
      Future<void> restore() async {
        await flow.initialize();
        if (!context.mounted) return;
        if (addingCommunity && flow.stage == HostedFlowStage.complete) {
          await flow.leave();
        }
        if (context.mounted) await flow.signInAccount(openBrowser: false);
      }

      unawaited(restore());
      return null;
    }, [flow]);
    final finishing = useRef(false);
    void finishEntry() {
      if (!addingCommunity || finishing.value) return;
      finishing.value = true;
      onComplete?.call();
      WidgetsBinding.instance.addPostFrameCallback((_) {
        if (!context.mounted) return;
        final route = ModalRoute.of(context);
        if (route != null && route.isActive) {
          route.navigator?.removeRoute(route);
        }
      });
    }

    ref.listen(communityEntryCompletionProvider, (_, _) => finishEntry());
    useEffect(() {
      final presentation = ref.read(hostedInlineEntryProvider.notifier);
      WidgetsBinding.instance.addPostFrameCallback((_) {
        if (context.mounted) presentation.setActive(true);
      });
      void completed() {
        if (flow?.stage == HostedFlowStage.complete) finishEntry();
      }

      flow?.addListener(completed);
      return () {
        flow?.removeListener(completed);
        scheduleMicrotask(() {
          presentation.setActive(false);
          if (addingCommunity &&
              onExit == null &&
              flow != null &&
              flow.stage != HostedFlowStage.complete) {
            unawaited(flow.leave());
          }
        });
      };
    }, [flow]);
    final signingIn = flow?.stage == HostedFlowStage.signIn;
    final busy =
        flow != null &&
        flow.accountOnly &&
        (flow.stage == HostedFlowStage.working ||
            flow.stage == HostedFlowStage.signIn);
    final account = flow?.account.account;
    final connected = account != null;
    final signedInLanding = connected && !addingCommunity && !signingIn;
    final email = account?.email?.trim();
    final hasKnownCommunities = useKnownCommunities(
      accountKey: flow?.account.account?.subject,
      active:
          !existingUser.value &&
          !enteringCommunityLink.value &&
          !creating.value &&
          flow?.stage == HostedFlowStage.idle &&
          (ModalRoute.of(context)?.isCurrent ?? true),
      load: () async =>
          (await loadKnownHostedCommunities(flow!.account)).isNotEmpty,
    );
    final problem = flow != null && flow.accountOnly ? flow.problem : null;
    final status = flow == null
        ? null
        : problem != null
        ? hostedFlowProblemMessage(problem)
        : busy
        ? null
        : connected
        ? email != null && email.isNotEmpty
              ? 'Signed in as $email'
              : 'Signed in to BuilderLab'
        : flow.noCommunitiesToAdd
        ? 'No communities available to add. Scan a QR code or use an invite link.'
        : null;
    final confirmingSignOut = useRef(false);
    Future<void> confirmSignOut() async {
      if (confirmingSignOut.value) return;
      confirmingSignOut.value = true;
      try {
        final confirmed = await showDestructiveConfirmation(
          context: context,
          title: 'Sign out of BuilderLab?',
          message:
              '${email != null && email.isNotEmpty ? 'You’re signed in as $email. ' : ''}'
              'Your saved Buzz identities will stay on this device.',
          confirmLabel: 'Sign out',
        );
        if (!context.mounted ||
            !confirmed ||
            flow?.account.account?.subject != account?.subject) {
          return;
        }
        await flow?.signOutAccount();
      } finally {
        confirmingSignOut.value = false;
      }
    }

    final handlingBack = useRef(false);
    void back() {
      final pairingStatus = ref.read(pairingProvider).status;
      if (pairingStatus == PairingStatus.transferring ||
          pairingStatus == PairingStatus.storing ||
          pairingStatus == PairingStatus.success) {
        return;
      }
      // Nested page and entry PopScopes receive the same system Back event.
      if (handlingBack.value) return;
      handlingBack.value = true;
      WidgetsBinding.instance.addPostFrameCallback((_) {
        handlingBack.value = false;
      });
      if (signedInLanding &&
          !busy &&
          !existingUser.value &&
          !enteringCommunityLink.value &&
          !creating.value) {
        unawaited(confirmSignOut());
        return;
      }
      if (!existingUser.value &&
          !enteringCommunityLink.value &&
          !creating.value &&
          (onExit != null || addingCommunity)) {
        if (onExit != null) {
          onExit!();
        } else {
          Navigator.of(context).pop();
        }
        return;
      }
      unawaited(flow?.leave());
      existingUser.value = false;
      creating.value = false;
      enteringCommunityLink.value = false;
      FocusManager.instance.primaryFocus?.unfocus();
      ref.read(pairingProvider.notifier).reset();
    }

    return PopScope(
      canPop:
          !signedInLanding &&
          onExit == null &&
          !existingUser.value &&
          !enteringCommunityLink.value &&
          !creating.value,
      onPopInvokedWithResult: (didPop, _) {
        if (!didPop &&
            onExit != null &&
            !existingUser.value &&
            !enteringCommunityLink.value &&
            !creating.value) {
          back();
          return;
        }
        if (!didPop &&
            (signedInLanding ||
                existingUser.value ||
                creating.value ||
                enteringCommunityLink.value)) {
          back();
        }
      },
      child: PairingPage(
        connectionLanding: true,
        showConnectionOptions:
            !HostedEnvironment.enabled || addingCommunity || connected,
        addingCommunity: addingCommunity,
        signedInOnboarding: connected && !addingCommunity && !signingIn,
        hasKnownCommunities: hasKnownCommunities,
        existingUser: existingUser.value,
        enteringCommunityLink: enteringCommunityLink.value,
        creatingCommunity: creating.value,
        onSubmitCommunity: flow == null
            ? null
            : (name) {
                creating.value = false;
                unawaited(flow.start(HostedCreateCommunity(name)));
              },
        onCommunityLink: () => enteringCommunityLink.value = true,
        onCreateCommunity: flow == null || busy
            ? null
            : () => creating.value = true,
        onBuilderlab: flow == null
            ? null
            : signingIn
            ? () => unawaited(flow.leave())
            : busy
            ? null
            : () => unawaited(flow.signInAccount()),
        onImportIdentity: flow == null || busy
            ? null
            : () => unawaited(flow.signInAccount(importIdentity: true)),
        accountStatus: status,
        builderlabBusy: busy && !signingIn,
        signInPending: signingIn || busy,
        builderlabLabel: signingIn
            ? 'Cancel sign-in'
            : busy
            ? 'Connecting…'
            : connected
            ? 'Continue with Builderlab'
            : 'Sign in with email',
        onWelcomeBack: back,
        onAlreadyUseBuzz: () => existingUser.value = true,
      ),
    );
  }
}
