part of '../settings_page.dart';

class _LanguageSection extends ConsumerWidget {
  const _LanguageSection();

  @override
  Widget build(BuildContext context, WidgetRef ref) {
    final localizations = AppLocalizations.of(context);
    final selected =
        ref.watch(localeProvider) ?? Localizations.localeOf(context);
    final value = selected.languageCode == 'pt'
        ? localizations.portugueseBrazil
        : localizations.englishUnitedStates;

    return AppListCard(
      label: localizations.languageAndRegion,
      children: [
        AppListRow(
          icon: LucideIcons.languages,
          title: localizations.language,
          value: value,
          trailing: const _RowChevron(),
          onTap: () => _showLanguageSheet(context),
        ),
      ],
    );
  }
}

void _showLanguageSheet(BuildContext context) {
  showModalBottomSheet<void>(
    context: context,
    showDragHandle: true,
    builder: (_) => const _LanguageSheet(),
  );
}

class _LanguageSheet extends ConsumerWidget {
  const _LanguageSheet();

  @override
  Widget build(BuildContext context, WidgetRef ref) {
    final localizations = AppLocalizations.of(context);
    final selected =
        ref.watch(localeProvider) ?? Localizations.localeOf(context);
    final options = [
      (locale: const Locale('pt', 'BR'), label: localizations.portugueseBrazil),
      (
        locale: const Locale('en', 'US'),
        label: localizations.englishUnitedStates,
      ),
    ];

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
              localizations.languageAndRegion,
              style: context.textTheme.titleMedium,
            ),
          ),
          for (final option in options)
            AppListRow(
              icon: LucideIcons.languages,
              title: option.label,
              trailing: option.locale.languageCode == selected.languageCode
                  ? Icon(
                      LucideIcons.check,
                      size: 18,
                      color: context.colors.primary,
                    )
                  : null,
              onTap: () {
                ref.read(localeProvider.notifier).setLocale(option.locale);
                Navigator.of(context).pop();
              },
            ),
          const SizedBox(height: Grid.xxs),
        ],
      ),
    );
  }
}
