part of '../settings_page.dart';

/// System / Light / Dark, mirroring desktop's appearance-mode selector.
const _modeOptions = <({ThemeMode mode, String label, IconData icon})>[
  (mode: ThemeMode.system, label: 'System', icon: LucideIcons.sunMoon),
  (mode: ThemeMode.light, label: 'Light', icon: LucideIcons.sun),
  (mode: ThemeMode.dark, label: 'Dark', icon: LucideIcons.moon),
];

String _modeLabel(BuildContext context, ThemeMode mode) {
  final l10n = AppLocalizations.of(context);
  return switch (mode) {
    ThemeMode.system => l10n.system,
    ThemeMode.light => l10n.light,
    ThemeMode.dark => l10n.dark,
  };
}

class _AppearanceSection extends ConsumerWidget {
  const _AppearanceSection();

  @override
  Widget build(BuildContext context, WidgetRef ref) {
    final mode = ref.watch(themeProvider);
    final schemeName = ref.watch(schemeProvider);
    final accentIndex = ref.watch(accentProvider);

    return AppListCard(
      label: AppLocalizations.of(context).styleSection,
      children: [
        AppListRow(
          icon: LucideIcons.sunMoon,
          title: AppLocalizations.of(context).appearance,
          value: _modeLabel(context, mode),
          trailing: const _RowChevron(),
          onTap: () => _showAppearanceModeSheet(context),
        ),
        AppListRow(
          icon: LucideIcons.palette,
          title: AppLocalizations.of(context).theme,
          value: themeSelectionLabel(schemeName, mode),
          trailing: const _RowChevron(),
          onTap: () => Navigator.of(context).push(
            MaterialPageRoute<void>(builder: (_) => const ThemePickerPage()),
          ),
        ),
        AppListRow(
          icon: LucideIcons.droplet,
          title: AppLocalizations.of(context).accentColor,
          // The swatch *is* the value — naming the color as well would say the
          // same thing twice, so it takes the chevron's place.
          trailing: _AccentSwatch(accentIndex: accentIndex),
          onTap: () => Navigator.of(context).push(
            MaterialPageRoute<void>(builder: (_) => const AccentPickerPage()),
          ),
        ),
      ],
    );
  }
}

void _showAppearanceModeSheet(BuildContext context) {
  showModalBottomSheet<void>(
    context: context,
    showDragHandle: true,
    builder: (_) => const _AppearanceModeSheet(),
  );
}

/// Three choices is too few to warrant a page, so the appearance row opens a
/// sheet rather than pushing a route.
class _AppearanceModeSheet extends ConsumerWidget {
  const _AppearanceModeSheet();

  @override
  Widget build(BuildContext context, WidgetRef ref) {
    final mode = ref.watch(themeProvider);

    return SafeArea(
      child: Column(
        mainAxisSize: MainAxisSize.min,
        crossAxisAlignment: CrossAxisAlignment.start,
        children: [
          Padding(
            padding: const EdgeInsets.fromLTRB(
              Grid.gutter,
              0,
              Grid.gutter,
              Grid.xxs,
            ),
            child: Text(
              AppLocalizations.of(context).appearance,
              style: context.textTheme.titleMedium,
            ),
          ),
          for (final option in _modeOptions)
            AppListRow(
              icon: option.icon,
              title: _modeLabel(context, option.mode),
              trailing: option.mode == mode
                  ? Icon(
                      LucideIcons.check,
                      size: 18,
                      color: context.colors.primary,
                    )
                  : null,
              onTap: () {
                final schemeName = ref.read(schemeProvider);
                final compatibleScheme = schemeForAppearanceMode(
                  schemeName,
                  option.mode,
                );
                if (compatibleScheme != schemeName) {
                  ref.read(schemeProvider.notifier).setScheme(compatibleScheme);
                }
                ref.read(themeProvider.notifier).setMode(option.mode);
                Navigator.of(context).pop();
              },
            ),
          const SizedBox(height: Grid.xxs),
        ],
      ),
    );
  }
}

/// The selected accent, shown where a row would otherwise carry a chevron.
class _AccentSwatch extends StatelessWidget {
  const _AccentSwatch({required this.accentIndex});

  static const _size = 22.0;

  final int accentIndex;

  @override
  Widget build(BuildContext context) {
    return Container(
      width: _size,
      height: _size,
      decoration: BoxDecoration(
        color: accentColorForScheme(context.colors, accentIndex),
        shape: BoxShape.circle,
        border: Border.all(color: context.colors.outlineVariant),
      ),
    );
  }
}
