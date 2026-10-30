import 'package:flutter/material.dart';

import '../../shared/widgets/modal_presentation.dart';
import 'channel_management_provider.dart';

const archiveChannelConfirmBody = 'The channel will become read-only.';

/// Whether [currentPubkey] may rename, archive, or unarchive a channel: an
/// owner or admin, or the human who owns an agent that owns the channel.
bool canManageChannelLifecycle({
  required List<ChannelMember> members,
  required Map<String, String> agentOwners,
  required String? currentPubkey,
}) {
  if (currentPubkey == null) return false;
  final me = currentPubkey.toLowerCase();
  final isElevated = members.any(
    (member) => member.pubkey.toLowerCase() == me && member.isElevated,
  );
  final ownsOwnerAgent = members.any(
    (member) =>
        member.isOwner &&
        agentOwners[member.pubkey.toLowerCase()]?.toLowerCase() == me,
  );
  return isElevated || ownsOwnerAgent;
}

/// Asks the user to confirm a channel lifecycle action. Returns true only when
/// they choose [confirmLabel].
Future<bool> confirmChannelAction(
  BuildContext context, {
  required String title,
  required String body,
  required String confirmLabel,
}) async {
  final confirmed = await showBuzzDialog<bool>(
    context: context,
    builder: (dialogContext) => AlertDialog(
      title: Text(title),
      content: Text(body),
      actions: [
        TextButton(
          onPressed: () => Navigator.of(dialogContext).pop(false),
          child: const Text('Cancel'),
        ),
        FilledButton(
          onPressed: () => Navigator.of(dialogContext).pop(true),
          child: Text(confirmLabel),
        ),
      ],
    ),
  );
  return confirmed == true;
}
