import 'package:flutter/material.dart';

import '../theme/theme.dart';

/// Shared text and inset action pill for temporary and persistent feedback.
class ToastCapsuleContent extends StatelessWidget {
  const ToastCapsuleContent({
    super.key,
    required this.message,
    required this.foreground,
    this.actionLabel = 'Retry',
    this.destructiveAction = false,
    this.secondaryAction,
    this.onAction,
    this.stackActions = false,
  });

  /// Places actions below the message when enlarged text needs more width.
  final bool stackActions;

  final String message;
  final Color foreground;
  final String actionLabel;

  /// Uses the theme error colors for the primary action.
  final bool destructiveAction;

  /// Adds a second recovery action and stacks both buttons below the message.
  final ({String label, VoidCallback onPressed})? secondaryAction;
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
    Widget action(
      String label,
      VoidCallback onPressed, {
      bool destructive = false,
      bool secondary = false,
    }) => FilledButton(
      onPressed: onPressed,
      style: FilledButton.styleFrom(
        backgroundColor: destructive
            ? lightColorScheme.error
            : secondary
            ? darkColorScheme.secondaryContainer
            : Colors.white,
        foregroundColor: destructive
            ? lightColorScheme.onError
            : secondary
            ? darkColorScheme.secondary
            : Colors.black,
        shape: const RoundedSuperellipseBorder(
          borderRadius: BorderRadius.all(Radius.circular(999)),
        ),
        minimumSize: const Size(48, 36),
        padding: const EdgeInsets.symmetric(horizontal: Grid.gutter),
        tapTargetSize: MaterialTapTargetSize.shrinkWrap,
      ),
      child: Text(label),
    );
    final secondary = secondaryAction;
    if (secondary != null || stackActions) {
      return Padding(
        padding: const EdgeInsets.fromLTRB(
          Grid.twelve,
          6,
          Grid.twelve,
          Grid.twelve,
        ),
        child: Column(
          mainAxisSize: MainAxisSize.min,
          crossAxisAlignment: CrossAxisAlignment.stretch,
          children: [
            text,
            action(actionLabel, onAction!, destructive: destructiveAction),
            if (secondary != null) ...[
              const SizedBox(height: 6),
              action(secondary.label, secondary.onPressed, secondary: true),
            ],
          ],
        ),
      );
    }
    return IntrinsicWidth(
      child: IntrinsicHeight(
        child: Row(
          mainAxisSize: MainAxisSize.min,
          crossAxisAlignment: CrossAxisAlignment.stretch,
          children: [
            Expanded(child: Center(widthFactor: 1, child: text)),
            Padding(
              padding: const EdgeInsets.only(top: 6, right: 6, bottom: 6),
              child: action(
                actionLabel,
                onAction!,
                destructive: destructiveAction,
              ),
            ),
          ],
        ),
      ),
    );
  }
}
