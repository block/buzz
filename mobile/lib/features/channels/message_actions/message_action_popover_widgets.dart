part of '../message_actions.dart';

class _MessageActionPreviewVisibility extends HookWidget {
  final bool visible;
  final ValueChanged<bool>? onChanged;
  final Widget child;

  const _MessageActionPreviewVisibility({
    required this.visible,
    required this.onChanged,
    required this.child,
  });

  @override
  Widget build(BuildContext context) {
    useEffect(() {
      WidgetsBinding.instance.addPostFrameCallback((_) {
        if (context.mounted) onChanged?.call(visible);
      });
      return null;
    }, [visible, onChanged]);
    return child;
  }
}

class _LiftedMessagePreview extends StatelessWidget {
  final ui.Image anchorSnapshot;

  const _LiftedMessagePreview({required this.anchorSnapshot});

  @override
  Widget build(BuildContext context) {
    return DecoratedBox(
      key: const ValueKey('message-action-preview'),
      decoration: BoxDecoration(
        color: context.colors.surfaceContainerHigh,
        borderRadius: BorderRadius.circular(Radii.md),
        border: Border.all(
          color: context.colors.outlineVariant.withValues(alpha: 0.7),
          width: 0.5,
        ),
        boxShadow: [
          BoxShadow(
            color: Colors.black.withValues(alpha: 0.18),
            blurRadius: 18,
            offset: const Offset(0, 8),
          ),
        ],
      ),
      child: Padding(
        padding: const EdgeInsets.all(_messageActionPreviewInset),
        child: ClipRRect(
          borderRadius: BorderRadius.circular(Radii.xs),
          child: RawImage(
            image: anchorSnapshot,
            fit: BoxFit.fill,
            filterQuality: FilterQuality.medium,
          ),
        ),
      ),
    );
  }
}

class _MessageActionSurface extends StatelessWidget {
  final List<_PopoverMessageAction> actions;
  final ValueChanged<String> onSelected;

  const _MessageActionSurface({
    required this.actions,
    required this.onSelected,
  });

  @override
  Widget build(BuildContext context) {
    final menuLayout = _MessageActionSurfaceLayout.from(context, actions);
    return Material(
      key: const ValueKey('message-action-surface'),
      color: context.colors.surface,
      surfaceTintColor: Colors.transparent,
      elevation: 10,
      shadowColor: Colors.black.withValues(alpha: 0.22),
      shape: RoundedRectangleBorder(
        borderRadius: BorderRadius.circular(Radii.dialog),
        side: BorderSide(
          color: context.colors.outlineVariant.withValues(alpha: 0.55),
          width: 0.5,
        ),
      ),
      clipBehavior: Clip.antiAlias,
      child: SingleChildScrollView(
        child: Padding(
          padding: const EdgeInsets.symmetric(
            vertical: _messageActionVerticalInset,
          ),
          child: Column(
            mainAxisSize: MainAxisSize.min,
            children: [
              for (var index = 0; index < actions.length; index++) ...[
                if (index > 0 &&
                    actions[index - 1].group != actions[index].group)
                  Divider(
                    key: ValueKey(
                      'message-action-divider-${actions[index].group.name}',
                    ),
                    height: _messageActionSeparatorHeight,
                    thickness: _messageActionSeparatorHeight,
                    indent: Grid.xs,
                    endIndent: Grid.xs,
                  ),
                _MessageActionRow(
                  action: actions[index],
                  height: menuLayout.rowHeight,
                  onSelected: onSelected,
                ),
              ],
            ],
          ),
        ),
      ),
    );
  }
}

class _MessageActionRow extends StatelessWidget {
  final _PopoverMessageAction action;
  final double height;
  final ValueChanged<String> onSelected;

  const _MessageActionRow({
    required this.action,
    required this.height,
    required this.onSelected,
  });

  @override
  Widget build(BuildContext context) {
    final foreground = action.destructive
        ? context.colors.error
        : context.colors.onSurface;
    return Semantics(
      button: true,
      label: action.title,
      excludeSemantics: true,
      child: InkWell(
        key: ValueKey('message-action-${action.id}'),
        onTap: () {
          unawaited(HapticFeedback.lightImpact());
          onSelected(action.id);
        },
        child: SizedBox(
          height: height,
          child: Padding(
            padding: const EdgeInsets.symmetric(horizontal: Grid.xs),
            child: Row(
              children: [
                SizedBox(
                  width: 32,
                  child: Center(
                    child: Icon(action.icon, size: 22, color: foreground),
                  ),
                ),
                const SizedBox(width: Grid.twelve),
                Expanded(
                  child: Text(
                    action.title,
                    maxLines: 1,
                    overflow: TextOverflow.ellipsis,
                    style: context.textTheme.bodyLarge?.copyWith(
                      color: foreground,
                    ),
                  ),
                ),
              ],
            ),
          ),
        ),
      ),
    );
  }
}

class _MessageActionSurfaceLayout {
  final double rowHeight, preferredHeight;

  const _MessageActionSurfaceLayout({
    required this.rowHeight,
    required this.preferredHeight,
  });

  factory _MessageActionSurfaceLayout.from(
    BuildContext context,
    List<_PopoverMessageAction> actions,
  ) {
    final textPainter = TextPainter(
      text: TextSpan(
        text: 'Message action',
        style: context.textTheme.bodyLarge,
      ),
      textDirection: Directionality.of(context),
      textScaler: MediaQuery.textScalerOf(context),
      maxLines: 1,
    )..layout();
    final rowHeight = math.max(
      _messageActionRowHeight,
      textPainter.height + (_messageActionRowVerticalPadding * 2),
    );
    textPainter.dispose();

    var separatorCount = 0;
    for (var index = 1; index < actions.length; index++) {
      if (actions[index - 1].group != actions[index].group) separatorCount += 1;
    }
    final preferredHeight =
        (_messageActionVerticalInset * 2) +
        (actions.length * rowHeight) +
        (separatorCount * _messageActionSeparatorHeight);
    return _MessageActionSurfaceLayout(
      rowHeight: rowHeight,
      preferredHeight: preferredHeight,
    );
  }
}
