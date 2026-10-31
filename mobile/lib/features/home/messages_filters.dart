import 'dart:async';

import 'package:flutter/material.dart';
import 'package:flutter/services.dart';

import '../../shared/theme/theme.dart';
import '../../shared/widgets/filter_chip_bar.dart';

class MessagesFilters extends StatelessWidget {
  const MessagesFilters({
    super.key,
    required this.selected,
    required this.onSelected,
  });
  final String selected;
  final ValueChanged<String> onSelected;

  @override
  Widget build(BuildContext context) => SizedBox(
    width: double.infinity,
    child: FilterChipBar<String>(
      useNativeGlass: true,
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
