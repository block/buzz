import 'package:flutter/material.dart';

import '../../../shared/me/me_sync.dart';

/// Offers an explicit choice for conflicting pending Solo organization edits.
Future<void> offerMeRecovery(BuildContext context, MeSync sync) async {
  final pending = sync.prefs?.getString(sync.pendingKey);
  if (pending == null) return;
  final discard = await showDialog<bool>(
    context: context,
    builder: (context) => AlertDialog(
      title: const Text('Conflicting Solo changes'),
      content: const Text(
        'Desktop and this phone changed the same project or chat. Keep the pending phone change to retry later, or discard it and use the current desktop organization. Your chat messages will be kept.',
      ),
      actions: [
        TextButton(
          onPressed: () => Navigator.pop(context, false),
          child: const Text('Keep pending'),
        ),
        TextButton(
          onPressed: () => Navigator.pop(context, true),
          child: const Text('Discard phone change'),
        ),
      ],
    ),
  );
  if (discard == true) await sync.discardPending(pending);
}
