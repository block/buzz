import 'dart:async';

import 'package:flutter/material.dart';
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
        route = PageRouteBuilder<void>(
          settings: const RouteSettings(name: 'hosted-setup'),
          transitionDuration: Duration.zero,
          reverseTransitionDuration: Duration.zero,
          pageBuilder: (_, _, _) => PopScope(
            canPop: false,
            onPopInvokedWithResult: (didPop, _) {
              if (!didPop) stepsNavigator.currentState?.maybePop();
            },
            child: Navigator(
              key: stepsNavigator,
              onGenerateRoute: (_) => MaterialPageRoute<void>(
                builder: (_) => _HostedSteps(
                  flow: flow,
                  onAddCommunity: onAddCommunity,
                  onClose: () => latest.value.onClose(),
                  profileBuilder: (context) =>
                      latest.value.profileBuilder(context),
                ),
              ),
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
  Widget build(BuildContext context) {
    useListenable(flow);
    final reducedMotion = MediaQuery.disableAnimationsOf(context);
    final presentedStage = flow.stage;
    final presentation = Object();
    final activePresentation = useRef<Object?>(null)..value = presentation;
    // Keep an opaque surface behind the native profile presentation and its
    // dismissal. Completion retires the route only after the flow settles.
    if (flow.stage == HostedFlowStage.profile) {
      return ColoredBox(
        color: context.colors.surface,
        child: profileBuilder(context),
      );
    }
    return ColoredBox(
      color: context.colors.surface,
      child: AnimatedSwitcher(
        duration: reducedMotion
            ? Duration.zero
            : const Duration(milliseconds: 250),
        switchInCurve: Curves.easeOutCubic,
        switchOutCurve: Curves.easeInCubic,
        layoutBuilder: (current, previous) => Stack(
          fit: StackFit.expand,
          children: [
            for (final child in previous)
              ExcludeFocus(
                child: ExcludeSemantics(child: IgnorePointer(child: child)),
              ),
            ?current,
          ],
        ),
        transitionBuilder: (child, animation) => SlideTransition(
          position: Tween(
            begin: const Offset(1, 0),
            end: Offset.zero,
          ).animate(animation),
          child: child,
        ),
        child: HostedConnectSheet(
          key: ValueKey(flow.stage),
          flow: flow,
          onAddCommunity: onAddCommunity,
          presentedStage: flow.stage,
          fullScreen: true,
          onBack: () {
            // Outgoing pages remain mounted until their transition finishes.
            if (activePresentation.value != presentation ||
                flow.stage != presentedStage) {
              return;
            }
            if (flow.canChangeCommunities) {
              flow.backToCommunityChoices();
            } else {
              onClose();
            }
          },
        ),
      ),
    );
  }
}
