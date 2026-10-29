import 'package:flutter/material.dart';
import 'package:flutter_hooks/flutter_hooks.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';

import '../../shared/hosted/hosted_flow.dart';
import '../../shared/theme/grid.dart';
import '../../shared/community/community_grid.dart';
import '../../shared/community/community_grid_option.dart';
import '../../shared/community/community_grid_add.dart';
import '../../onboarding_app_entry.dart';
import '../../shared/widgets/onboarding_step_page.dart';

/// Community selection is completed before admission reveals Home.
class HostedCommunitySelection extends HookConsumerWidget {
  const HostedCommunitySelection({
    super.key,
    required this.flow,
    this.onAddCommunity,
    this.fullScreen = false,
    this.onBack,
  });
  final HostedFlow flow;
  final VoidCallback? onAddCommunity;
  final bool fullScreen;
  final VoidCallback? onBack;

  @override
  Widget build(BuildContext context, WidgetRef ref) {
    useListenable(flow);
    Future<void> addCommunity() async {
      final navigator = Navigator.of(context, rootNavigator: true);
      if (!await flow.prepareToAddCommunity() || !navigator.mounted) return;
      late final MaterialPageRoute<void> route;
      var exiting = false;
      Future<void> exit() async {
        if (exiting) return;
        exiting = true;
        await flow.signInAccount();
        if (flow.stage == HostedFlowStage.signIn ||
            flow.stage == HostedFlowStage.idle) {
          exiting = false;
          return;
        }
        if (route.isActive) route.navigator?.removeRoute(route);
      }

      route = MaterialPageRoute<void>(
        builder: (_) => OnboardingAppEntry(
          addingCommunity: true,
          restoreAccount: false,
          onExit: exit,
        ),
      );
      navigator.push(route);
      // Mount the opaque entry route before retiring the setup cover.
      WidgetsBinding.instance.addPostFrameCallback((_) {
        if (route.isActive) flow.leave();
      });
    }

    final choices = CommunityGrid(
      children: [
        for (final community in flow.communities)
          CommunityGridOption(
            id: community.relayUrl,
            name: community.name,
            relayUrl: community.relayUrl,
            onSelect: (_) async {
              flow.chooseLandingCommunity(community.relayUrl);
              await flow.addSelectedCommunities();
            },
          ),
        CommunityGridAdd(onTap: onAddCommunity ?? addCommunity),
      ],
    );
    if (fullScreen) {
      return OnboardingStepPage(
        title: 'Select a community',
        onBack: onBack ?? () {},
        content: choices,
      );
    }
    return Column(
      crossAxisAlignment: CrossAxisAlignment.stretch,
      children: [
        choices,
        const SizedBox(height: Grid.sm),
      ],
    );
  }
}
