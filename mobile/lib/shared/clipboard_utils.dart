import 'package:flutter/material.dart';
import 'package:flutter/services.dart';

import 'widgets/buzz_toast.dart';

Future<void> copyToClipboard(
  BuildContext context,
  String text, {
  String message = 'Copied',
}) async {
  await Clipboard.setData(ClipboardData(text: text));
  if (!context.mounted) return;
  showBuzzToast(context, message, kind: ToastKind.success);
}
