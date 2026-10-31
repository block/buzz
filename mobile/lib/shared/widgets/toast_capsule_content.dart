import 'package:flutter/material.dart';

import '../theme/theme.dart';

/// Shared text and inset action pill for temporary and persistent feedback.
class ToastCapsuleContent extends StatelessWidget {
  const ToastCapsuleContent({
    super.key,
    required this.message,
    required this.foreground,
    this.actionLabel = 'Retry',
    this.onAction,
  });

  final String message;
  final Color foreground;
  final String actionLabel;
  final VoidCallback? onAction;

  @override
  Widget build(BuildContext context) {
    final text = Padding(
      padding: const EdgeInsets.symmetric(
        horizontal: Grid.gutter,
        vertical: Grid.twelve,
      ),
      child: Text(
        message,
        textAlign: TextAlign.center,
        style: context.textTheme.bodyMedium?.copyWith(
          color: foreground,
          fontWeight: FontWeight.w600,
        ),
      ),
    );
    if (onAction == null) return text;
    return IntrinsicWidth(
      child: IntrinsicHeight(
        child: Row(
          mainAxisSize: MainAxisSize.min,
          crossAxisAlignment: CrossAxisAlignment.stretch,
          children: [
            Expanded(child: Center(widthFactor: 1, child: text)),
            Padding(
              padding: const EdgeInsets.only(top: 6, right: 6, bottom: 6),
              child: FilledButton(
                onPressed: onAction,
                style: FilledButton.styleFrom(
                  backgroundColor: Colors.white,
                  foregroundColor: Colors.black,
                  shape: const RoundedSuperellipseBorder(
                    borderRadius: BorderRadius.all(Radius.circular(999)),
                  ),
                  minimumSize: const Size(48, 36),
                  padding: const EdgeInsets.symmetric(horizontal: Grid.gutter),
                  tapTargetSize: MaterialTapTargetSize.shrinkWrap,
                ),
                child: Text(actionLabel),
              ),
            ),
          ],
        ),
      ),
    );
  }
}
