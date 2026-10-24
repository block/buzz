import 'dart:async';
import 'package:flutter/material.dart';
import 'package:flutter_hooks/flutter_hooks.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';

import 'features/pairing/pairing_page.dart';
import 'features/onboarding/hosted_connect_sheet.dart';
import 'features/pairing/pairing_provider.dart';
import 'shared/hosted/hosted_flow_provider.dart';
import 'shared/hosted/hosted_flow.dart';
import 'shared/hosted/hosted_environment.dart';
import 'shared/hosted/hosted_community_creation.dart';
import 'shared/hosted/hosted_flow_problem_message.dart';

/// Navigation over the existing branded welcome surface.
class OnboardingAppEntry extends HookConsumerWidget {
  const OnboardingAppEntry({super.key, this.addingCommunity = false});
  final bool addingCommunity;

  @override
  Widget build(BuildContext context, WidgetRef ref) {
    final existingUser = useState(false);
    final creating = useState(false);
    final enteringCommunityLink = useState(false);
    final flow = HostedEnvironment.enabled
        ? ref.watch(hostedFlowProvider)
        : null;
    useListenable(flow);
    useEffect(() {
      if (flow == null) return null;
      Future<void> restore() async {
        await flow.initialize();
        if (context.mounted) await flow.signInAccount(openBrowser: false);
      }

      unawaited(restore());
      return null;
    }, [flow]);
    void finishEntry() {
      if (!addingCommunity) return;
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
              flow != null &&
              flow.stage != HostedFlowStage.complete) {
            unawaited(flow.leave());
          }
        });
      };
    }, [flow]);
    final inlineSetup =
        flow != null &&
        !flow.accountOnly &&
        flow.stage != HostedFlowStage.signIn &&
        !(flow.stage == HostedFlowStage.working &&
            flow.account.account == null) &&
        flow.stage != HostedFlowStage.idle &&
        flow.stage != HostedFlowStage.complete &&
        flow.stage != HostedFlowStage.profile &&
        flow.stage != HostedFlowStage.chooseIdentity &&
        flow.stage != HostedFlowStage.chooseCommunities &&
        flow.stage != HostedFlowStage.communityConflict &&
        !flow.canChangeCommunities;
    final signingIn = flow?.stage == HostedFlowStage.signIn;
    final busy =
        flow != null &&
        flow.accountOnly &&
        (flow.stage == HostedFlowStage.working ||
            flow.stage == HostedFlowStage.signIn);
    final connected = flow?.account.account != null;
    final problem = flow != null && flow.accountOnly ? flow.problem : null;
    final status = flow == null
        ? null
        : problem != null
        ? hostedFlowProblemMessage(problem)
        : busy
        ? null
        : flow.noCommunitiesToAdd
        ? 'No communities available to add. Scan a QR code or use an invite link.'
        : connected
        ? 'Connected to Builderlab.'
        : null;
    void back() {
      unawaited(flow?.leave());
      existingUser.value = false;
      creating.value = false;
      enteringCommunityLink.value = false;
      FocusManager.instance.primaryFocus?.unfocus();
      ref.read(pairingProvider.notifier).reset();
    }

    return PopScope(
      canPop:
          !existingUser.value &&
          !enteringCommunityLink.value &&
          !creating.value &&
          !inlineSetup,
      onPopInvokedWithResult: (didPop, _) {
        if (!didPop &&
            (existingUser.value ||
                creating.value ||
                enteringCommunityLink.value ||
                inlineSetup)) {
          back();
        }
      },
      child: PairingPage(
        connectionLanding: true,
        showConnectionOptions:
            !HostedEnvironment.enabled || addingCommunity || connected,
        addingCommunity: addingCommunity,
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
        accountStatus: status,
        builderlabBusy: busy,
        setupContent: flow != null && inlineSetup
            ? Column(
                mainAxisSize: MainAxisSize.min,
                children: [
                  HostedConnectSheet(flow: flow),
                  TextButton(onPressed: back, child: const Text('Leave setup')),
                ],
              )
            : null,
        builderlabLabel: signingIn
            ? 'Cancel sign-in'
            : busy
            ? 'Connecting…'
            : connected
            ? 'Continue with Builderlab'
            : 'Sign in with builderlab',
        onWelcomeBack: back,
        onAlreadyUseBuzz: () => existingUser.value = true,
      ),
    );
  }
}
