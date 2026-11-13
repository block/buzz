part of '../compose_bar.dart';

class _FormattingToolbar extends StatelessWidget {
  final void Function(String prefix, [String? suffix]) onFormat;

  const _FormattingToolbar({required this.onFormat});

  @override
  Widget build(BuildContext context) {
    return Row(
      key: const ValueKey('formatting-actions'),
      mainAxisSize: MainAxisSize.min,
      children: [
        _FormatButton(
          icon: BuzzIcons.bold,
          tooltip: 'Bold',
          onTap: () => onFormat('**'),
        ),
        _FormatButton(
          icon: BuzzIcons.italic,
          tooltip: 'Italic',
          onTap: () => onFormat('_'),
        ),
        _FormatButton(
          icon: BuzzIcons.strikethrough,
          tooltip: 'Strikethrough',
          onTap: () => onFormat('~~'),
        ),
        _FormatButton(
          icon: BuzzIcons.code,
          tooltip: 'Code',
          onTap: () => onFormat('`'),
        ),
        _FormatButton(
          icon: BuzzIcons.squareCode,
          tooltip: 'Code block',
          onTap: () => onFormat('```\n', '\n```'),
        ),
      ],
    );
  }
}

class _FormatButton extends StatelessWidget {
  final IconData icon;
  final String tooltip;
  final VoidCallback onTap;

  const _FormatButton({
    required this.icon,
    required this.tooltip,
    required this.onTap,
  });

  @override
  Widget build(BuildContext context) {
    return Tooltip(
      message: tooltip,
      child: InkWell(
        borderRadius: BorderRadius.circular(Radii.sm),
        onTap: () => _runComposerAction(onTap),
        child: Padding(
          padding: const EdgeInsets.all(Grid.xxs),
          child: Icon(icon, size: 18, color: context.colors.primary),
        ),
      ),
    );
  }
}

class _ComposeAction extends StatelessWidget {
  final IconData icon;
  final VoidCallback onTap;

  const _ComposeAction({required this.icon, required this.onTap});

  @override
  Widget build(BuildContext context) {
    return SizedBox(
      width: 36,
      height: 36,
      child: IconButton(
        onPressed: () => _runComposerAction(onTap),
        icon: Icon(icon, size: 20, color: context.colors.onSurfaceVariant),
        padding: EdgeInsets.zero,
        visualDensity: VisualDensity.compact,
      ),
    );
  }
}

/// Wraps (or inserts) markdown formatting around the current selection.
void _applyMarkdownFormat(
  _MarkdownEditingController controller,
  FocusNode focusNode,
  ObjectRef<bool> isModifyingText,
  String prefix, [
  String? suffix,
]) {
  suffix ??= prefix;
  final text = controller.text;
  final sel = controller.selection;
  if (!sel.isValid) return;

  isModifyingText.value = true;
  try {
    if (sel.isCollapsed) {
      final offset = sel.baseOffset;
      final updated =
          '${text.substring(0, offset)}$prefix$suffix${text.substring(offset)}';
      controller.text = updated;
      controller.selection = TextSelection.collapsed(
        offset: offset + prefix.length,
      );
    } else {
      final selected = text.substring(sel.start, sel.end);
      final updated =
          '${text.substring(0, sel.start)}$prefix$selected$suffix${text.substring(sel.end)}';
      controller.text = updated;
      controller.selection = TextSelection.collapsed(
        offset: sel.start + prefix.length + selected.length + suffix.length,
      );
    }
  } finally {
    isModifyingText.value = false;
  }
  focusNode.requestFocus();
}
