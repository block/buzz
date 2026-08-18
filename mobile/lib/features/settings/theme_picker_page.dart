import 'package:flutter/material.dart';
import 'package:flutter_hooks/flutter_hooks.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';
import 'package:lucide_icons_flutter/lucide_icons.dart';

import '../../shared/theme/theme.dart';
import '../../shared/widgets/frosted_app_bar.dart';
import '../../shared/widgets/frosted_scaffold.dart';

/// Themes for the current appearance mode, organised the way desktop does it:
/// System offers the light/dark pairs as single entries, while Light and Dark
/// each list every theme of that brightness.
class ThemePickerPage extends HookConsumerWidget {
  const ThemePickerPage({super.key});

  // ListTile height (~56px) used for scroll offset calculation.
  static const _itemHeight = 56.0;

  @override
  Widget build(BuildContext context, WidgetRef ref) {
    final preference = ref.watch(communityThemeProvider);
    final mode = preference.mode;
    final selectedScheme = preference.theme;
    final searchQuery = useState('');
    final searchController = useTextEditingController();
    final scrollController = useScrollController();

    final groups = themeGroups();
    final entries = switch (mode) {
      ThemeMode.system => groups.paired,
      ThemeMode.light => groups.light,
      ThemeMode.dark => groups.dark,
    };
    final isSystem = mode == ThemeMode.system;

    String labelFor(ThemeColors theme) =>
        isSystem ? pairedThemeLabel(theme.name) : theme.displayName;

    final query = searchQuery.value.toLowerCase();
    final filtered = query.isEmpty
        ? entries
        : entries
              .where((t) => labelFor(t).toLowerCase().contains(query))
              .toList();

    // In System mode either half of a pair counts as the selection; in the
    // pinned modes the effective theme already accounts for coercion, so the
    // checkmark always names what is actually rendered.
    final active = effectiveTheme(selectedScheme, mode);
    bool isSelected(ThemeColors theme) {
      if (active == null) return false;
      if (!isSystem) return active.name == theme.name;
      return active.name == theme.name ||
          themePairFor(theme.name) == active.name;
    }

    // Auto-scroll to the selected theme on first build (no search active).
    useEffect(() {
      if (query.isNotEmpty) return null;
      final idx = filtered.indexWhere(isSelected);
      if (idx < 0) return null;
      final offset = idx * _itemHeight;
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
      appBar: const FrostedAppBar(title: Text('Theme')),
      body: Column(
        children: [
          SizedBox(height: frostedAppBarHeight(context)),
          _SearchField(
            controller: searchController,
            query: searchQuery.value,
            onChanged: (v) => searchQuery.value = v,
            onClear: () {
              searchController.clear();
              searchQuery.value = '';
            },
          ),
          Expanded(
            child: filtered.isEmpty
                ? Padding(
                    padding: const EdgeInsets.all(Grid.sm),
                    child: Center(
                      child: Text(
                        'No themes found',
                        style: context.textTheme.bodyMedium?.copyWith(
                          color: context.colors.onSurfaceVariant,
                        ),
                      ),
                    ),
                  )
                : ListView.builder(
                    controller: scrollController,
                    itemCount: filtered.length,
                    itemBuilder: (_, i) {
                      final theme = filtered[i];
                      final pairName = isSystem
                          ? themePairFor(theme.name)
                          : null;
                      return _ThemeRow(
                        theme: theme,
                        pair: pairName == null ? null : findTheme(pairName),
                        label: labelFor(theme),
                        selected: isSelected(theme),
                        onTap: () => ref
                            .read(communityThemeProvider.notifier)
                            .setTheme(theme.name),
                      );
                    },
                  ),
          ),
        ],
      ),
    );
  }
}

class _SearchField extends StatelessWidget {
  const _SearchField({
    required this.controller,
    required this.query,
    required this.onChanged,
    required this.onClear,
  });

  final TextEditingController controller;
  final String query;
  final ValueChanged<String> onChanged;
  final VoidCallback onClear;

  @override
  Widget build(BuildContext context) {
    return Padding(
      padding: const EdgeInsets.symmetric(
        horizontal: Grid.gutter,
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
                controller: controller,
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
                onChanged: onChanged,
              ),
            ),
            if (query.isNotEmpty)
              GestureDetector(
                onTap: onClear,
                child: Icon(
                  LucideIcons.x,
                  size: 16,
                  color: context.colors.onSurfaceVariant,
                ),
              ),
          ],
        ),
      ),
    );
  }
}

/// A theme row with a colour bar + name + checkmark. When [pair] is supplied the
/// bar is split down the middle — light stripes on the left, dark on the right —
/// standing in for desktop's split light/dark preview tile.
class _ThemeRow extends StatelessWidget {
  const _ThemeRow({
    required this.theme,
    required this.pair,
    required this.label,
    required this.selected,
    required this.onTap,
  });

  final ThemeColors theme;
  final ThemeColors? pair;
  final String label;
  final bool selected;
  final VoidCallback onTap;

  @override
  Widget build(BuildContext context) {
    final swatch = pair == null ? [theme] : [theme, pair!];

    return ListTile(
      leading: Container(
        width: 56,
        height: 28,
        decoration: BoxDecoration(
          borderRadius: BorderRadius.circular(Radii.sm),
          border: Border.all(color: context.colors.outlineVariant),
        ),
        child: ClipRRect(
          borderRadius: BorderRadius.circular(Radii.sm - 1),
          child: Row(
            children: [
              for (final t in swatch)
                for (final color in [t.bg, t.fg, t.comment])
                  Expanded(
                    child: ColoredBox(
                      color: color,
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
