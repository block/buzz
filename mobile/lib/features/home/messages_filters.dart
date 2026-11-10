import 'dart:async';

import 'package:flutter/material.dart';
import 'package:flutter/foundation.dart';
import 'package:flutter/services.dart';

import '../../shared/theme/theme.dart';
import '../../shared/widgets/filter_chip_bar.dart';

class MessagesFilters extends StatelessWidget {
  const MessagesFilters({
    super.key,
    this.isCollapsed = false,
    required this.selected,
    required this.onSelected,
  });
  final bool isCollapsed;
  final String selected;
  final ValueChanged<String> onSelected;

  @override
  Widget build(BuildContext context) => SizedBox(
    width: double.infinity,
    child: FilterChipBar<String>(
      useNativeGlass: true,
      matchNativeWidths: true,
      labelOffset: defaultTargetPlatform == TargetPlatform.android
          ? const Offset(-0.5, -1)
          : Offset.zero,
      side: defaultTargetPlatform == TargetPlatform.android && isCollapsed
          ? BorderSide(color: context.colors.primary.withValues(alpha: 0.02))
          : BorderSide.none,
      minControlHeight: defaultTargetPlatform == TargetPlatform.iOS ? 44 : 48,
      horizontalPadding: Grid.gutter,
      selected: selected,
      onSelected: (value) {
        unawaited(HapticFeedback.selectionClick());
        onSelected(value);
      },
      visualDensity: const VisualDensity(horizontal: -2),
      chipVerticalPadding: Grid.xxs,
      barVerticalPadding: Grid.half,
      items: [
        const FilterChipItem(id: 'All', label: 'All'),
        const FilterChipItem(id: 'Solo', label: 'Solo'),
        const FilterChipItem(id: 'Team', label: 'Team'),
      ],
    ),
  );
}
