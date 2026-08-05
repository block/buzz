import 'package:flutter/material.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';
import 'package:lucide_icons_flutter/lucide_icons.dart';

import '../../shared/theme/theme.dart';
import '../../shared/widgets/frosted_app_bar.dart';
import '../../shared/widgets/frosted_scaffold.dart';

/// Accent colors as a list of rows, matching the theme picker's shape. The
/// swatches used to live inline on the settings page as a wrap of tappable
/// circles; a row per color keeps the settings page to plain rows and gives each
/// accent a readable name.
class AccentPickerPage extends ConsumerWidget {
  const AccentPickerPage({super.key});

  @override
  Widget build(BuildContext context, WidgetRef ref) {
    final selected =
        accentIndexForWireValue(ref.watch(communityThemeProvider).accent) ??
        defaultAccentIndex;
    final colorScheme = context.colors;

    return FrostedScaffold(
      appBar: const FrostedAppBar(title: Text('Accent Color')),
      body: ListView(
        padding: EdgeInsets.only(
          top: frostedAppBarHeight(context),
          bottom: Grid.xs,
        ),
        children: [
          for (var i = 0; i < accentColors.length; i++)
            _AccentRow(
              color: accentColorForScheme(colorScheme, i),
              label: accentColors[i].name,
              selected: selected == i,
              onTap: () =>
                  ref.read(communityThemeProvider.notifier).setAccent(i),
            ),
        ],
      ),
    );
  }
}

class _AccentRow extends StatelessWidget {
  const _AccentRow({
    required this.color,
    required this.label,
    required this.selected,
    required this.onTap,
  });

  final Color color;
  final String label;
  final bool selected;
  final VoidCallback onTap;

  @override
  Widget build(BuildContext context) {
    return ListTile(
      leading: Container(
        width: 28,
        height: 28,
        decoration: BoxDecoration(
          color: color,
          shape: BoxShape.circle,
          border: Border.all(color: context.colors.outlineVariant),
        ),
      ),
      title: Text(label),
      trailing: selected
          ? Icon(LucideIcons.check, size: 18, color: context.colors.primary)
          : null,
      onTap: onTap,
    );
  }
}
