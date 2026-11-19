import 'package:flutter/material.dart';

import '../../shared/widgets/modal_presentation.dart';

/// Confirms a channel lifecycle action without closing its presenting page.
Future<bool> showChannelActionConfirmation({
  required BuildContext context,
  required String title,
  required String body,
  required String confirmLabel,
}) async =>
    await showBuzzDialog<bool>(
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
    ) ==
    true;
