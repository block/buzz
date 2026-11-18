part of '../activity_page.dart';

class _ActivityMenuButton<T> extends StatelessWidget {
  const _ActivityMenuButton({
    required this.value,
    required this.options,
    required this.onChanged,
    required this.controlKey,
    required this.popoverKey,
    required this.semanticLabel,
    required this.active,
  });
  final T value;
  final Map<T, String> options;
  final ValueChanged<T> onChanged;
  final String controlKey, popoverKey, semanticLabel;
  final bool active;
  String get label => options[value]!;

  @override
  Widget build(BuildContext context) {
    if (defaultTargetPlatform == TargetPlatform.iOS) {
      final textScaler = MediaQuery.textScalerOf(context);
      final height = max(48.0, textScaler.scale(16) * 1.4 + 16);
      final labelWidth = (TextPainter(
        text: TextSpan(
          text: label,
          style: filterChipTextStyle.copyWith(fontWeight: FontWeight.w600),
        ),
        textScaler: textScaler,
        textDirection: Directionality.of(context),
      )..layout()).width;
      return Padding(
        padding: const EdgeInsets.symmetric(vertical: 4),
        child: IosMessageCreateButton(
          key: ValueKey(controlKey),
          label: label,
          labelFontSize: 16,
          icon: 'chevronDown',
          showIconWithLabel: true,
          eagerGestures: false,
          accessibilityLabel: semanticLabel,
          width: labelWidth + 56,
          height: height - 8,
          plain: false,
          fillWidth: true,
          deferMenuActions: false,
          selectedIndex: options.keys.toList().indexOf(value),
          actions: [
            for (final entry in options.entries)
              (
                label: entry.value,
                symbol: '',
                onPressed: () => onChanged(entry.key),
              ),
          ],
        ),
      );
    }
    return Builder(
      builder: (buttonContext) => TextButton(
        key: ValueKey(controlKey),
        style: _activityFilterStyle(context, active),
        onPressed: () async {
          final selected = await showAnchoredPopover<T>(
            context: buttonContext,
            width: 240,
            alignment: AnchoredPopoverAlignment.start,
            surfaceKey: ValueKey(popoverKey),
            items: [
              for (final entry in options.entries)
                CheckedPopupMenuItem(
                  value: entry.key,
                  checked: value == entry.key,
                  height: Grid.xl,
                  child: Text(entry.value),
                ),
            ],
          );
          if (buttonContext.mounted && selected != null) onChanged(selected);
        },
        child: Semantics(
          label: semanticLabel,
          selected: active,
          child: Row(
            mainAxisSize: MainAxisSize.min,
            mainAxisAlignment: MainAxisAlignment.center,
            children: [
              Flexible(
                child: Text(
                  label,
                  maxLines: 1,
                  overflow: TextOverflow.ellipsis,
                ),
              ),
              const SizedBox(width: Grid.quarter),
              const Icon(BuzzIcons.chevronDown, size: 16),
            ],
          ),
        ),
      ),
    );
  }
}

class _UnreadFilterButton extends StatelessWidget {
  const _UnreadFilterButton({required this.selected, required this.onPressed});
  final bool selected;
  final VoidCallback onPressed;
  @override
  Widget build(BuildContext context) {
    if (defaultTargetPlatform == TargetPlatform.iOS) {
      final scaler = MediaQuery.textScalerOf(context);
      final height = max(48.0, scaler.scale(16) * 1.4 + 16);
      final width =
          (TextPainter(
            text: TextSpan(
              text: 'Unread',
              style: filterChipTextStyle.copyWith(fontWeight: FontWeight.w600),
            ),
            textScaler: scaler,
            textDirection: Directionality.of(context),
          )..layout()).width +
          32;
      return IosGlassNavigationButton(
        key: const ValueKey('activity-unread-filter'),
        icon: IosGlassNavigationIcon.more,
        label: 'Unread',
        semanticLabel: 'Unread',
        isSelected: selected,
        onPressed: onPressed,
        width: width,
        height: height,
        controlSize: height - 8,
        fillWidth: true,
        foregroundColor: context.colors.primary,
      );
    }
    return Semantics(
      toggled: selected,
      child: TextButton(
        key: const ValueKey('activity-unread-filter'),
        style: _activityFilterStyle(context, selected),
        onPressed: onPressed,
        child: const Text('Unread'),
      ),
    );
  }
}

ButtonStyle _activityFilterStyle(BuildContext context, bool active) =>
    TextButton.styleFrom(
      minimumSize: Size(
        0,
        max(44.0, MediaQuery.textScalerOf(context).scale(16) * 1.4 + 12),
      ),
      padding: const EdgeInsets.symmetric(horizontal: 12),
      shape: const StadiumBorder(),
      textStyle: filterChipTextStyle.copyWith(
        leadingDistribution: TextLeadingDistribution.even,
      ),
      visualDensity: VisualDensity.standard,
      tapTargetSize: MaterialTapTargetSize.padded,
      foregroundColor: active
          ? context.colors.primary
          : context.colors.onSurface,
      backgroundColor: active
          ? context.colors.primaryContainer
          : context.controlSurface,
    );
