import 'package:flutter/material.dart';

import '../theme/theme.dart';

/// Hairline divider between action groups in a bottom sheet, matching the
/// `AppListSection` divider convention.
class SheetDivider extends StatelessWidget {
  const SheetDivider({super.key});

  @override
  Widget build(BuildContext context) {
    return Padding(
      padding: const EdgeInsets.symmetric(vertical: Grid.xxs),
      child: Divider(height: 1, color: context.colors.outlineVariant),
    );
  }
}
