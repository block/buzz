part of '../compose_bar.dart';

class _FormattingToolbar extends StatelessWidget {
  final void Function(String prefix, [String? suffix]) onFormat;

  const _FormattingToolbar({required this.onFormat});

  @override
  Widget build(BuildContext context) {
    return Padding(
      padding: const EdgeInsets.only(bottom: Grid.half),
      child: Row(
        children: [
          _FormatButton(
            icon: LucideIcons.bold,
            tooltip: 'Bold',
            onTap: () => onFormat('**'),
          ),
          _FormatButton(
            icon: LucideIcons.italic,
            tooltip: 'Italic',
            onTap: () => onFormat('_'),
          ),
          _FormatButton(
            icon: LucideIcons.strikethrough,
            tooltip: 'Strikethrough',
            onTap: () => onFormat('~~'),
          ),
          _FormatButton(
            icon: LucideIcons.code,
            tooltip: 'Code',
            onTap: () => onFormat('`'),
          ),
          _FormatButton(
            icon: LucideIcons.squareCode,
            tooltip: 'Code block',
            onTap: () => onFormat('```\n', '\n```'),
          ),
        ],
      ),
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
        onTap: onTap,
        child: Padding(
          padding: const EdgeInsets.all(Grid.xxs),
          child: Icon(icon, size: 18, color: context.colors.onSurfaceVariant),
        ),
      ),
    );
  }
}

class _ComposeAction extends StatelessWidget {
  final IconData icon;
  final bool active;
  final VoidCallback onTap;

  const _ComposeAction({
    required this.icon,
    this.active = false,
    required this.onTap,
  });

  @override
  Widget build(BuildContext context) {
    return SizedBox(
      width: 36,
      height: 36,
      child: IconButton(
        onPressed: onTap,
        icon: Icon(
          icon,
          size: 20,
          color: active
              ? context.colors.primary
              : context.colors.onSurfaceVariant,
        ),
        padding: EdgeInsets.zero,
        visualDensity: VisualDensity.compact,
      ),
    );
  }
}
