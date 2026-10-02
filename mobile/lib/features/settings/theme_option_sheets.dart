import 'dart:async';

import 'package:flutter/material.dart';
import 'package:flutter/services.dart';
import 'package:flutter_hooks/flutter_hooks.dart';
import 'package:lucide_icons_flutter/lucide_icons.dart';

import '../../shared/theme/theme.dart';
import '../../shared/widgets/modal_presentation.dart';

const appearanceModeOptions = <({ThemeMode mode, String label, IconData icon})>[
  (mode: ThemeMode.light, label: 'Light', icon: LucideIcons.sun),
  (mode: ThemeMode.dark, label: 'Dark', icon: LucideIcons.moon),
  (mode: ThemeMode.system, label: 'System', icon: LucideIcons.sunMoon),
];

String appearanceModeLabel(ThemeMode mode) =>
    appearanceModeOptions.firstWhere((option) => option.mode == mode).label;

Future<void> showAppearanceModePickerSheet({
  required BuildContext context,
  required ThemeMode selectedMode,
  required ValueChanged<ThemeMode> onSelected,
}) => showBuzzModalBottomSheet<void>(
  context: context,
  title: 'Appearance',
  showDragHandle: true,
  builder: (_) =>
      _AppearanceModePicker(initialMode: selectedMode, onSelected: onSelected),
);

Future<void> showAccentColorPickerSheet({
  required BuildContext context,
  required ColorScheme colorScheme,
  required int selectedIndex,
  required ValueChanged<int> onSelected,
}) => showBuzzModalBottomSheet<void>(
  context: context,
  title: 'Accent color',
  showDragHandle: true,
  builder: (_) => _AccentColorPicker(
    colorScheme: colorScheme,
    initialIndex: selectedIndex,
    onSelected: onSelected,
  ),
);

class _AppearanceModePicker extends HookWidget {
  const _AppearanceModePicker({
    required this.initialMode,
    required this.onSelected,
  });

  final ThemeMode initialMode;
  final ValueChanged<ThemeMode> onSelected;

  @override
  Widget build(BuildContext context) {
    final selected = useState(initialMode);
    return SafeArea(
      top: false,
      child: Padding(
        padding: const EdgeInsets.fromLTRB(
          Grid.gutter,
          Grid.xxs,
          Grid.gutter,
          Grid.xs,
        ),
        child: SizedBox(
          height: 132,
          child: Row(
            crossAxisAlignment: CrossAxisAlignment.stretch,
            children: [
              for (
                var index = 0;
                index < appearanceModeOptions.length;
                index++
              ) ...[
                if (index > 0) const SizedBox(width: Grid.xxs),
                Expanded(
                  child: _AppearanceModeOption(
                    option: appearanceModeOptions[index],
                    selected:
                        appearanceModeOptions[index].mode == selected.value,
                    onTap: () {
                      final mode = appearanceModeOptions[index].mode;
                      unawaited(HapticFeedback.selectionClick());
                      selected.value = mode;
                      onSelected(mode);
                    },
                  ),
                ),
              ],
            ],
          ),
        ),
      ),
    );
  }
}

class _AppearanceModeOption extends StatelessWidget {
  const _AppearanceModeOption({
    required this.option,
    required this.selected,
    required this.onTap,
  });

  final ({ThemeMode mode, String label, IconData icon}) option;
  final bool selected;
  final VoidCallback onTap;

  @override
  Widget build(BuildContext context) => Semantics(
    button: true,
    selected: selected,
    label: '${option.label} appearance',
    child: Container(
      key: ValueKey('appearance-mode-${option.mode.name}'),
      decoration: BoxDecoration(
        color: context.colors.surfaceContainerHighest,
        borderRadius: BorderRadius.circular(Radii.container),
      ),
      foregroundDecoration: BoxDecoration(
        borderRadius: BorderRadius.circular(Radii.container),
        border: selected
            ? Border.all(color: context.colors.primary, width: 2)
            : null,
      ),
      child: Material(
        color: Colors.transparent,
        borderRadius: BorderRadius.circular(Radii.container),
        clipBehavior: Clip.antiAlias,
        child: InkWell(
          onTap: onTap,
          child: Stack(
            alignment: Alignment.center,
            children: [
              Transform.translate(
                offset: const Offset(0, -Grid.twelve),
                child: Icon(
                  option.icon,
                  key: ValueKey('appearance-mode-${option.mode.name}-icon'),
                  size: 34,
                  color: selected
                      ? context.colors.primary
                      : context.colors.onSurface,
                ),
              ),
              Positioned(
                left: Grid.xxs,
                right: Grid.xxs,
                bottom: Grid.xxs,
                child: Text(
                  option.label,
                  key: ValueKey('appearance-mode-${option.mode.name}-label'),
                  textAlign: TextAlign.center,
                  style: context.textTheme.labelLarge?.copyWith(
                    color: selected
                        ? context.colors.primary
                        : context.colors.onSurface,
                    fontWeight: FontWeight.w600,
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

class _AccentColorPicker extends HookWidget {
  const _AccentColorPicker({
    required this.colorScheme,
    required this.initialIndex,
    required this.onSelected,
  });

  final ColorScheme colorScheme;
  final int initialIndex;
  final ValueChanged<int> onSelected;

  @override
  Widget build(BuildContext context) {
    final selected = useState(initialIndex);
    return SafeArea(
      top: false,
      child: Padding(
        padding: const EdgeInsets.fromLTRB(
          Grid.gutter,
          Grid.xxs,
          Grid.gutter,
          Grid.xs,
        ),
        child: GridView.builder(
          key: const ValueKey('accent-selection-grid'),
          shrinkWrap: true,
          primary: false,
          padding: EdgeInsets.zero,
          gridDelegate: const SliverGridDelegateWithFixedCrossAxisCount(
            crossAxisCount: 4,
            crossAxisSpacing: Grid.xxs,
            mainAxisSpacing: Grid.xs,
          ),
          itemCount: accentColors.length,
          itemBuilder: (context, index) => _AccentSheetOption(
            index: index,
            color: accentColorForScheme(colorScheme, index),
            selected: selected.value == index,
            onTap: () {
              unawaited(HapticFeedback.selectionClick());
              selected.value = index;
              onSelected(index);
            },
          ),
        ),
      ),
    );
  }
}

class _AccentSheetOption extends StatelessWidget {
  const _AccentSheetOption({
    required this.index,
    required this.color,
    required this.selected,
    required this.onTap,
  });

  final int index;
  final Color color;
  final bool selected;
  final VoidCallback onTap;

  @override
  Widget build(BuildContext context) => Semantics(
    label: '${accentColors[index].name} accent color',
    button: true,
    selected: selected,
    child: InkResponse(
      key: ValueKey('accent-option-$index'),
      onTap: onTap,
      radius: 36,
      child: Center(
        child: Container(
          key: ValueKey('accent-swatch-$index'),
          width: 64,
          height: 64,
          padding: const EdgeInsets.all(5),
          decoration: BoxDecoration(
            color: color,
            shape: BoxShape.circle,
            border: Border.all(color: context.colors.outlineVariant),
          ),
          child: selected
              ? DecoratedBox(
                  decoration: BoxDecoration(
                    shape: BoxShape.circle,
                    border: Border.all(
                      color: color.computeLuminance() > 0.55
                          ? const Color(0xFF111111)
                          : Colors.white,
                      width: 3,
                    ),
                  ),
                )
              : null,
        ),
      ),
    ),
  );
}
