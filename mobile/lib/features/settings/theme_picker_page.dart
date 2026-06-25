import 'package:flutter/material.dart';
import 'package:flutter_hooks/flutter_hooks.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';
import 'package:lucide_icons_flutter/lucide_icons.dart';

import '../../shared/theme/theme.dart';
import '../../shared/widgets/frosted_app_bar.dart';
import '../../shared/widgets/frosted_scaffold.dart';

class ThemePickerPage extends HookConsumerWidget {
  const ThemePickerPage({super.key});

  // ListTile height (~56px) used for scroll offset calculation.
  static const _itemHeight = 56.0;

  @override
  Widget build(BuildContext context, WidgetRef ref) {
    final selectedScheme = ref.watch(schemeProvider);
    final searchQuery = useState('');
    final searchController = useTextEditingController();
    final scrollController = useScrollController();

    // Flat alphabetical list by display name
    final sorted = List<ThemeColors>.from(themeCatalog)
      ..sort((a, b) => a.displayName.compareTo(b.displayName));

    final query = searchQuery.value.toLowerCase();
    final filtered = query.isEmpty
        ? sorted
        : sorted
              .where((t) => t.displayName.toLowerCase().contains(query))
              .toList();
    final showDefault =
        query.isEmpty ||
        'default'.contains(query) ||
        'catppuccin'.contains(query);

    // Default entry colors based on current brightness
    final isLight = context.colors.brightness == Brightness.light;
    final defaultBg = isLight
        ? const Color(0xFFEFF1F5)
        : const Color(0xFF24273A);
    final defaultFg = isLight
        ? const Color(0xFF4C4F69)
        : const Color(0xFFCAD3F5);
    final defaultComment = isLight
        ? const Color(0xFF7C7F93)
        : const Color(0xFF939AB7);

    // Auto-scroll to the selected theme on first build (no search active)
    useEffect(() {
      if (selectedScheme == null || query.isNotEmpty) return null;
      final idx = sorted.indexWhere((t) => t.name == selectedScheme);
      if (idx < 0) return null;
      // +1 to account for the Default entry at the top
      final offset = (idx + 1) * _itemHeight;
      WidgetsBinding.instance.addPostFrameCallback((_) {
        if (scrollController.hasClients) {
          scrollController.animateTo(
            offset.clamp(0.0, scrollController.position.maxScrollExtent),
            duration: const Duration(milliseconds: 300),
            curve: Curves.easeOut,
          );
        }
      });
      return null;
    }, const []);

    return FrostedScaffold(
      appBar: const FrostedAppBar(title: Text('Color Scheme')),
      body: Column(
        children: [
          SizedBox(height: frostedAppBarHeight(context)),
          // Always-visible search bar
          Padding(
            padding: const EdgeInsets.symmetric(
              horizontal: Grid.xs,
              vertical: Grid.xxs,
            ),
            child: Container(
              height: 36,
              padding: const EdgeInsets.symmetric(horizontal: Grid.twelve),
              decoration: BoxDecoration(
                color: context.colors.surfaceContainerHighest,
                borderRadius: BorderRadius.circular(Radii.lg),
                border: Border.all(color: context.colors.outlineVariant),
              ),
              child: Row(
                children: [
                  Icon(
                    LucideIcons.search,
                    size: 16,
                    color: context.colors.onSurfaceVariant,
                  ),
                  const SizedBox(width: Grid.xxs),
                  Expanded(
                    child: TextField(
                      controller: searchController,
                      decoration: InputDecoration(
                        hintText: 'Search themes...',
                        hintStyle: context.textTheme.bodyMedium?.copyWith(
                          color: context.colors.onSurfaceVariant,
                        ),
                        border: InputBorder.none,
                        enabledBorder: InputBorder.none,
                        focusedBorder: InputBorder.none,
                        isDense: true,
                        contentPadding: EdgeInsets.zero,
                      ),
                      style: context.textTheme.bodyMedium,
                      onChanged: (v) => searchQuery.value = v,
                    ),
                  ),
                  if (searchQuery.value.isNotEmpty)
                    GestureDetector(
                      onTap: () {
                        searchController.clear();
                        searchQuery.value = '';
                      },
                      child: Icon(
                        LucideIcons.x,
                        size: 16,
                        color: context.colors.onSurfaceVariant,
                      ),
                    ),
                ],
              ),
            ),
          ),

          // Theme list
          Expanded(
            child: ListView(
              controller: scrollController,
              children: [
                // Default entry
                if (showDefault)
                  _ThemeRow(
                    bg: defaultBg,
                    fg: defaultFg,
                    comment: defaultComment,
                    label: 'Default (Catppuccin)',
                    selected: selectedScheme == null,
                    onTap: () =>
                        ref.read(schemeProvider.notifier).setScheme(null),
                  ),

                // All themes, flat alphabetical
                for (final theme in filtered)
                  _ThemeRow(
                    bg: theme.bg,
                    fg: theme.fg,
                    comment: theme.comment,
                    label: theme.displayName,
                    selected: selectedScheme == theme.name,
                    onTap: () =>
                        ref.read(schemeProvider.notifier).setScheme(theme.name),
                  ),

                if (!showDefault && filtered.isEmpty)
                  Padding(
                    padding: const EdgeInsets.all(Grid.sm),
                    child: Center(
                      child: Text(
                        'No themes found',
                        style: context.textTheme.bodyMedium?.copyWith(
                          color: context.colors.onSurfaceVariant,
                        ),
                      ),
                    ),
                  ),
              ],
            ),
          ),
        ],
      ),
    );
  }
}

/// A row showing a 3-stripe color bar (bg/fg/comment) + theme name + checkmark.
class _ThemeRow extends StatelessWidget {
  const _ThemeRow({
    required this.bg,
    required this.fg,
    required this.comment,
    required this.label,
    required this.selected,
    required this.onTap,
  });

  final Color bg;
  final Color fg;
  final Color comment;
  final String label;
  final bool selected;
  final VoidCallback onTap;

  @override
  Widget build(BuildContext context) {
    return ListTile(
      leading: Container(
        width: 56,
        height: 28,
        decoration: BoxDecoration(
          borderRadius: BorderRadius.circular(Radii.sm),
          border: Border.all(color: context.colors.outlineVariant, width: 1),
        ),
        child: ClipRRect(
          borderRadius: BorderRadius.circular(Radii.sm - 1),
          child: Row(
            children: [
              Expanded(
                child: ColoredBox(color: bg, child: const SizedBox.expand()),
              ),
              Expanded(
                child: ColoredBox(color: fg, child: const SizedBox.expand()),
              ),
              Expanded(
                child: ColoredBox(
                  color: comment,
                  child: const SizedBox.expand(),
                ),
              ),
            ],
          ),
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
