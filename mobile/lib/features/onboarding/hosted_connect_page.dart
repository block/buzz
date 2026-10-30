import 'dart:async';

import 'package:flutter/material.dart';
import '../../shared/widgets/onboarding_route.dart';
import 'package:flutter_hooks/flutter_hooks.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';

import '../../shared/hosted/hosted_flow.dart';
import '../../shared/theme/theme_extensions.dart';
import 'hosted_connect_sheet.dart';

/// Owns a full-screen route above both initial entry and pushed Add Community.
class HostedConnectPage extends HookConsumerWidget {
  const HostedConnectPage({
    super.key,
    required this.flow,
    this.onAddCommunity,
    required this.onClose,
    required this.profileBuilder,
  });
  final HostedFlow flow;
  final VoidCallback? onAddCommunity;
  final VoidCallback onClose;
  final WidgetBuilder profileBuilder;

  @override
  Widget build(BuildContext context, WidgetRef ref) {
    final latest = useRef(this)..value = this;
    useEffect(() {
      var active = true;
      Route<void>? route;
      final stepsNavigator = GlobalKey<NavigatorState>();
      WidgetsBinding.instance.addPostFrameCallback((_) {
        if (!active || !context.mounted) return;
        route = onboardingRoute<void>(
          context,
          settings: const RouteSettings(name: 'hosted-setup'),
          builder: (_) => PopScope(
            canPop: false,
            onPopInvokedWithResult: (didPop, _) {
              if (!didPop) stepsNavigator.currentState?.maybePop();
            },
            child: _HostedSteps(
              navigatorKey: stepsNavigator,
              flow: flow,
              onAddCommunity: onAddCommunity,
              onClose: () => latest.value.onClose(),
              profileBuilder: (context) => latest.value.profileBuilder(context),
            ),
          ),
        );
        unawaited(
          Navigator.of(context, rootNavigator: true).push(route!).then((_) {
            if (active &&
                flow.stage != HostedFlowStage.complete &&
                flow.stage != HostedFlowStage.idle) {
              latest.value.onClose();
            }
          }),
        );
      });
      return () {
        active = false;
        WidgetsBinding.instance.addPostFrameCallback((_) {
          final current = route;
          if (current != null && current.isActive) {
            current.navigator?.removeRoute(current);
          }
        });
      };
    }, [flow]);
    return ColoredBox(color: context.colors.surface);
  }
}

class _HostedSteps extends HookWidget {
  const _HostedSteps({
    required this.navigatorKey,
    required this.flow,
    this.onAddCommunity,
    required this.onClose,
    required this.profileBuilder,
  });
  final GlobalKey<NavigatorState> navigatorKey;
  final HostedFlow flow;
  final VoidCallback? onAddCommunity;
  final VoidCallback onClose;
  final WidgetBuilder profileBuilder;

  @override
  Widget build(BuildContext context) {
    useListenable(flow);
    final presentedStage = flow.stage;
    final presentation = Object();
    final activePresentation = useRef<Object?>(null)..value = presentation;
    // Native profile owns its entry animation; keep its underlying page stable.
    final pageStage = useRef(flow.stage);
    if (flow.stage != HostedFlowStage.profile) pageStage.value = flow.stage;
    void goBack() {
      if (activePresentation.value != presentation ||
          flow.stage != presentedStage) {
        return;
      }
      if (flow.canChangeCommunities) {
        flow.backToCommunityChoices();
      } else {
        onClose();
      }
    }

    return Navigator(
      key: navigatorKey,
      onDidRemovePage: (_) {},
      pages: [
        OnboardingPage<void>(
          key: ValueKey(pageStage.value),
          child: PopScope(
            canPop: false,
            onPopInvokedWithResult: (didPop, _) {
              if (!didPop) goBack();
            },
            child: flow.stage == HostedFlowStage.profile
                ? ColoredBox(
                    color: context.colors.surface,
                    child: profileBuilder(context),
                  )
                : HostedConnectSheet(
                    flow: flow,
                    onAddCommunity: onAddCommunity,
                    presentedStage: presentedStage,
                    fullScreen: true,
                    onBack: goBack,
                  ),
          ),
        ),
      ],
    );
  }
}
