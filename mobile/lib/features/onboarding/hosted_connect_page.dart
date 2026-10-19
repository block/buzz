import 'package:flutter/material.dart';
import 'package:flutter_hooks/flutter_hooks.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';

import '../../shared/hosted/hosted_flow.dart';
import '../../shared/widgets/modal_presentation.dart';
import 'hosted_connect_sheet.dart';

/// Presents setup with the same modal chrome as the app's other sheets.
class HostedConnectPage extends HookConsumerWidget {
  const HostedConnectPage({
    super.key,
    required this.flow,
    required this.onClose,
  });
  final HostedFlow flow;
  final VoidCallback onClose;

  @override
  Widget build(BuildContext context, WidgetRef ref) {
    useEffect(() {
      var active = true;
      ModalRoute<dynamic>? sheetRoute;
      WidgetsBinding.instance.addPostFrameCallback((_) async {
        if (!active || !context.mounted) return;
        while (active &&
            context.mounted &&
            flow.stage != HostedFlowStage.complete &&
            flow.stage != HostedFlowStage.idle) {
          if (!context.mounted) return;
          await showBuzzModalBottomSheet<void>(
            context: context,
            title: 'Connect to Builderlab',
            useRootNavigator: true,
            isScrollControlled: true,
            useSafeArea: true,
            builder: (sheetContext) {
              sheetRoute = ModalRoute.of(sheetContext);
              return HostedConnectSheet(flow: flow);
            },
          );
          if (!active || !context.mounted) return;
          if (flow.stage != HostedFlowStage.profile) {
            onClose();
            return;
          }
          // Dismissing an optional profile is a skip. Re-present the continuing
          // batch so a later error or choice never ends up behind a closed sheet.
          flow.finishProfile();
          await WidgetsBinding.instance.endOfFrame;
        }
      });
      return () {
        active = false;
        final route = sheetRoute;
        WidgetsBinding.instance.addPostFrameCallback((_) {
          if (route != null && route.isActive) {
            route.navigator?.removeRoute(route);
          }
        });
      };
    }, [flow]);
    return const SizedBox.shrink();
  }
}
