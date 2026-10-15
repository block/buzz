import 'package:flutter/foundation.dart';
import 'package:flutter/material.dart';
import 'package:flutter/services.dart';

import '../theme/theme.dart';
import 'modal_presentation.dart';

/// Confirms a destructive action using a UIKit alert on iOS.
/// Returns true only when the person explicitly chooses the destructive action.
Future<bool> showDestructiveConfirmation({
  required BuildContext context,
  required String title,
  required String message,
  required String confirmLabel,
}) async {
  if (!kIsWeb && defaultTargetPlatform == TargetPlatform.iOS) {
    return await const MethodChannel(
          'buzz/confirmation_dialog',
        ).invokeMethod<bool>('present', {
          'title': title,
          'message': message,
          'confirmLabel': confirmLabel,
          'cancelLabel': 'Cancel',
          'dark': context.theme.brightness == Brightness.dark,
        }) ==
        true;
  }
  return await showBuzzDialog<bool>(
        context: context,
        builder: (dialogContext) => AlertDialog.adaptive(
          title: Text(title),
          content: Text(message),
          actions: [
            TextButton(
              onPressed: () => Navigator.of(dialogContext).pop(false),
              child: const Text('Cancel'),
            ),
            FilledButton(
              onPressed: () => Navigator.of(dialogContext).pop(true),
              style: FilledButton.styleFrom(
                backgroundColor: context.colors.error,
              ),
              child: Text(confirmLabel),
            ),
          ],
        ),
      ) ==
      true;
}
