import 'dart:async';

import 'package:flutter/material.dart';
import 'package:flutter/services.dart';
import 'package:flutter_hooks/flutter_hooks.dart';
import 'package:buzz/shared/theme/buzz_icons.dart';

import '../../shared/theme/theme.dart';
import '../../shared/widgets/modal_presentation.dart';

const appearanceModeOptions = <({ThemeMode mode, String label, IconData icon})>[
  (mode: ThemeMode.light, label: 'Light', icon: BuzzIcons.sun),
  (mode: ThemeMode.dark, label: 'Dark', icon: BuzzIcons.moon),
  (mode: ThemeMode.system, label: 'System', icon: BuzzIcons.sunMoon),
];

String appearanceModeLabel(ThemeMode mode) =>
    appearanceModeOptions.firstWhere((option) => option.mode == mode).label;

Future<void> showAccentColorPickerSheet({
  required BuildContext context,
  required ColorScheme colorScheme,
  required int selectedIndex,
  required ValueChanged<int> onSelected,
}) => showBuzzModalBottomSheet<void>(
  context: context,
  title: 'Chat color',
  showDragHandle: true,
  builder: (_) => _AccentColorPicker(
    colorScheme: colorScheme,
    initialIndex: selectedIndex,
    onSelected: onSelected,
  ),
);

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
          itemCount: chatColorChoices.length,
          itemBuilder: (context, index) => _AccentSheetOption(
            index: index,
            color: chatColorForScheme(colorScheme, index),
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
    label: '${chatColorChoices[index].name} chat color',
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
