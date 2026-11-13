import 'package:flutter/material.dart';

import '../../../shared/widgets/modal_presentation.dart';
import 'agent_activity_sheet.dart';

/// Opens the live agent activity transcript for [agentPubkey] in [channelId].
///
/// Shared by the channel app bar shortcut and the members sheet so both entry
/// points stay identical (#3907 discoverability).
Future<void> showAgentActivitySheet({
  required BuildContext context,
  required String channelId,
  required String agentPubkey,
}) {
  return showBuzzModalBottomSheet<void>(
    context: context,
    isScrollControlled: true,
    showDragHandle: true,
    builder: (_) => AgentActivitySheet(
      channelId: channelId,
      agentPubkey: agentPubkey,
    ),
  );
}
