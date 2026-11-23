part of '../compose_bar.dart';

class _ComposerBuzzLinkChip extends StatelessWidget {
  final IconData icon;
  final String label;
  final String semanticLabel;
  final TextStyle textStyle;

  const _ComposerBuzzLinkChip({
    required this.icon,
    required this.label,
    required this.semanticLabel,
    required this.textStyle,
  });

  @override
  Widget build(BuildContext context) {
    final style = textStyle.copyWith(
      color: context.colors.primary,
      fontWeight: FontWeight.w600,
      height: 1,
    );
    final fontSize = style.fontSize ?? 16;
    return Semantics(
      label: semanticLabel,
      excludeSemantics: true,
      child: Container(
        key: ValueKey('composer-buzz-link-chip:$label'),
        padding: const EdgeInsets.symmetric(horizontal: 4, vertical: 1),
        decoration: BoxDecoration(
          color: context.colors.primary.withValues(alpha: 0.16),
          borderRadius: BorderRadius.circular(Radii.sm),
          border: Border.all(
            color: context.colors.primary.withValues(alpha: 0.12),
          ),
        ),
        child: Row(
          mainAxisSize: MainAxisSize.min,
          children: [
            Icon(icon, size: fontSize * 0.95, color: context.colors.primary),
            const SizedBox(width: Grid.quarter + 1),
            Text(label, style: style),
          ],
        ),
      ),
    );
  }
}

/// One selected mention: a person (`@` prefix) or an agent (bot icon).
/// Semantics name the chip once, so a screen reader has one stop per
/// mention rather than its hidden source characters.
class _ComposerMentionChip extends StatelessWidget {
  final String label;
  final bool isAgent;
  final TextStyle textStyle;

  const _ComposerMentionChip({
    required this.label,
    required this.isAgent,
    required this.textStyle,
  });

  @override
  Widget build(BuildContext context) {
    final style = textStyle.copyWith(
      color: context.colors.primary,
      fontWeight: FontWeight.w500,
      height: 1,
    );
    final fontSize = style.fontSize ?? 16;

    return Semantics(
      label: isAgent ? 'Agent mention: $label' : 'Mention: $label',
      excludeSemantics: true,
      child: Container(
        key: ValueKey(
          isAgent
              ? 'composer-agent-mention-chip'
              : 'composer-person-mention-chip',
        ),
        padding: const EdgeInsets.fromLTRB(
          Grid.half,
          Grid.quarter + 1,
          Grid.half,
          Grid.quarter,
        ),
        decoration: BoxDecoration(
          // The composer surface is already tinted, so the body chip's
          // low-opacity fill disappears here. Keep the same chip geometry
          // while giving this editable token enough contrast to read as one.
          color: context.colors.primary.withValues(alpha: 0.16),
          borderRadius: BorderRadius.circular(Radii.sm),
          border: Border.all(
            color: context.colors.primary.withValues(alpha: 0.12),
          ),
        ),
        child: Row(
          mainAxisSize: MainAxisSize.min,
          crossAxisAlignment: CrossAxisAlignment.center,
          children: [
            if (isAgent) ...[
              Icon(
                BuzzIcons.bot,
                size: fontSize * 0.95,
                color: context.colors.primary,
              ),
              const SizedBox(width: Grid.quarter),
              Text(label, style: style),
            ] else
              Text('@$label', style: style),
          ],
        ),
      ),
    );
  }
}

/// Routes user edits through [_MarkdownEditingController.formatUserEdit] so
/// composer chips follow their editing rules.
class _ComposerChipEditingFormatter extends TextInputFormatter {
  final _MarkdownEditingController controller;

  _ComposerChipEditingFormatter(this.controller);

  @override
  TextEditingValue formatEditUpdate(
    TextEditingValue oldValue,
    TextEditingValue newValue,
  ) => controller.formatUserEdit(oldValue, newValue);
}

/// Marks Undo and Redo for [_MarkdownEditingController].
/// `UndoHistory` lets an ancestor override its undo and redo actions; this
/// one tells the controller which way the history is moving while the
/// default action restores the value. The formatter can then tell a
/// restored value from a key press that happens to land on earlier text,
/// and the controller restores the bindings it held with that text, so the
/// first of two namesake chips comes back with its own recipient.
class _ComposerHistoryAction<T extends Intent> extends Action<T> {
  final _MarkdownEditingController controller;

  _ComposerHistoryAction(this.controller);

  @override
  bool isEnabled(T intent, [BuildContext? context]) =>
      callingAction?.isEnabled(intent) ?? false;

  @override
  Object? invoke(T intent, [BuildContext? context]) {
    final history = callingAction;
    if (history == null) return null;
    controller._historyDirection = intent is RedoTextIntent ? 1 : -1;
    try {
      return history.invoke(intent);
    } finally {
      controller._historyDirection = 0;
    }
  }
}
