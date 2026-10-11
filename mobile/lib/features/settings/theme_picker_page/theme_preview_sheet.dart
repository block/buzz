part of '../theme_picker_page.dart';

const _appearanceOrder = [ThemeMode.light, ThemeMode.dark, ThemeMode.system];
const _appearanceMotionDuration = Duration(milliseconds: 150);
const _appearanceMotionCurve = Cubic(0.23, 1, 0.32, 1);

class _ThemePreviewExperience extends HookConsumerWidget {
  const _ThemePreviewExperience();

  @override
  Widget build(BuildContext context, WidgetRef ref) {
    final stored = ref.read(communityThemeProvider);
    final initialEntries = _themeEntriesForMode(stored.mode);
    var initialIndex = initialEntries.indexWhere(
      (entry) => _themeEntryIsSelected(entry, stored.mode, stored.theme),
    );
    if (initialIndex < 0) initialIndex = 0;

    final draftMode = useState(stored.mode);
    final draftThemeName = useState(initialEntries[initialIndex].name);
    final draftAccent = useState(
      accentIndexForWireValue(stored.accent) ?? defaultAccentIndex,
    );
    final currentPage = useState(initialIndex);
    final suppressNextPageHaptic = useRef(false);
    final controller = usePageController(initialPage: initialIndex);
    final entries = _themeEntriesForMode(draftMode.value);
    final platformBrightness = MediaQuery.platformBrightnessOf(context);
    final reduceMotion = MediaQuery.disableAnimationsOf(context);
    final previousDraftMode = usePrevious(draftMode.value);
    final animatePaginationChanges =
        previousDraftMode == null || previousDraftMode == draftMode.value;

    var selectedIndex = entries.indexWhere(
      (entry) =>
          _themeEntryIsSelected(entry, draftMode.value, draftThemeName.value),
    );
    if (selectedIndex < 0) selectedIndex = 0;
    final selectedEntry = entries[selectedIndex];
    final selectedDisplayed = _displayedThemeFor(
      selectedEntry,
      draftMode.value,
      platformBrightness,
    );
    final selectedResolved = resolveSchemes(
      selectedEntry.name,
      draftMode.value,
    );
    final selectedBaseScheme = platformBrightness == Brightness.dark
        ? selectedResolved.dark
        : selectedResolved.light;
    final supportsAccent = !isBuzzTheme(selectedDisplayed.name);

    void updateMode(ThemeMode mode) {
      final normalized = mode == ThemeMode.system
          ? schemeForAppearanceMode(draftThemeName.value, mode)
          : effectiveTheme(draftThemeName.value, mode)?.name;
      final nextEntries = _themeEntriesForMode(mode);
      final nextTheme = normalized ?? nextEntries.first.name;
      var nextIndex = nextEntries.indexWhere(
        (entry) => _themeEntryIsSelected(entry, mode, nextTheme),
      );
      if (nextIndex < 0) nextIndex = 0;

      draftMode.value = mode;
      draftThemeName.value = nextEntries[nextIndex].name;
      if (nextIndex != currentPage.value) {
        suppressNextPageHaptic.value = true;
      }
      currentPage.value = nextIndex;
      WidgetsBinding.instance.addPostFrameCallback((_) {
        if (controller.hasClients) controller.jumpToPage(nextIndex);
      });
    }

    void cycleAppearance() {
      final index = _appearanceOrder.indexOf(draftMode.value);
      final next = _appearanceOrder[(index + 1) % _appearanceOrder.length];
      unawaited(HapticFeedback.selectionClick());
      updateMode(next);
    }

    void applySelection() {
      unawaited(HapticFeedback.mediumImpact());
      final notifier = ref.read(communityThemeProvider.notifier);
      final currentAccent = ref.read(communityThemeProvider).accent;
      notifier.setPreference(
        CommunityThemePreference(
          theme: selectedEntry.name,
          accent: supportsAccent
              ? accentColors[draftAccent.value].wireValue
              : currentAccent,
          followSystem: draftMode.value == ThemeMode.system,
        ),
      );
      Navigator.of(context).pop();
    }

    void close() {
      unawaited(HapticFeedback.lightImpact());
      Navigator.of(context).pop();
    }

    return FrostedScaffold(
      useUtilitySurfaceTheme: true,
      appBar: FrostedAppBar(
        automaticallyImplyLeading: false,
        centerTitle: true,
        showBottomDivider: false,
        nativeLeading: IosNavigationAction(
          label: 'Close',
          symbol: 'xmark',
          onPressed: close,
        ),
        nativeActions: [
          IosNavigationAction(label: 'Set', onPressed: applySelection),
        ],
        leading: _ThemePreviewCloseButton(onPressed: close),
        title: const Text('Appearance'),
        actions: [_ThemePreviewSetButton(onPressed: applySelection)],
      ),
      body: Column(
        children: [
          SizedBox(height: frostedAppBarHeight(context)),
          Expanded(
            child: PageView.builder(
              key: const ValueKey('theme-preview-pages'),
              controller: controller,
              itemCount: entries.length,
              onPageChanged: (index) {
                if (suppressNextPageHaptic.value) {
                  suppressNextPageHaptic.value = false;
                } else if (index != currentPage.value) {
                  unawaited(HapticFeedback.selectionClick());
                }
                currentPage.value = index;
                draftThemeName.value = entries[index].name;
              },
              itemBuilder: (context, index) => AnimatedBuilder(
                animation: controller,
                builder: (context, _) {
                  var titleOffset = 0.0;
                  if (!reduceMotion && controller.hasClients) {
                    final page = controller.page;
                    if (page != null) {
                      titleOffset = (index - page).clamp(-1.0, 1.0) * Grid.md;
                    }
                  }
                  return _ThemePreviewPage(
                    theme: entries[index],
                    mode: draftMode.value,
                    accentIndex: draftAccent.value,
                    platformBrightness: platformBrightness,
                    titleHorizontalOffset: titleOffset,
                  );
                },
              ),
            ),
          ),
          SafeArea(
            top: false,
            minimum: const EdgeInsets.only(bottom: Grid.xs),
            child: Padding(
              padding: const EdgeInsets.fromLTRB(
                Grid.gutter,
                Grid.twelve,
                Grid.gutter,
                0,
              ),
              child: Row(
                children: [
                  SizedBox(
                    width: 64,
                    child: IgnorePointer(
                      ignoring: !supportsAccent,
                      child: AnimatedSwitcher(
                        key: const ValueKey(
                          'theme-preview-accent-availability',
                        ),
                        duration: reduceMotion
                            ? Duration.zero
                            : const Duration(milliseconds: 180),
                        reverseDuration: reduceMotion
                            ? Duration.zero
                            : const Duration(milliseconds: 140),
                        switchInCurve: _appearanceMotionCurve,
                        switchOutCurve: Curves.easeInOutCubic,
                        layoutBuilder: (currentChild, previousChildren) =>
                            Stack(
                              alignment: Alignment.center,
                              children: [...previousChildren, ?currentChild],
                            ),
                        transitionBuilder: (child, animation) => FadeTransition(
                          opacity: animation,
                          child: ScaleTransition(
                            scale: Tween(
                              begin: 0.78,
                              end: 1.0,
                            ).animate(animation),
                            child: child,
                          ),
                        ),
                        child: supportsAccent
                            ? _PreviewSheetCircleAction(
                                key: const ValueKey(
                                  'theme-preview-accent-action-button',
                                ),
                                semanticLabel: 'Accent color',
                                color: accentColorForScheme(
                                  selectedBaseScheme,
                                  draftAccent.value,
                                ),
                                onTap: () {
                                  unawaited(HapticFeedback.lightImpact());
                                  showAccentColorPickerSheet(
                                    context: context,
                                    colorScheme: selectedBaseScheme,
                                    selectedIndex: draftAccent.value,
                                    onSelected: (index) =>
                                        draftAccent.value = index,
                                  );
                                },
                              )
                            : const SizedBox(
                                key: ValueKey(
                                  'theme-preview-accent-unavailable',
                                ),
                              ),
                      ),
                    ),
                  ),
                  Expanded(
                    child: PageIndicator(
                      semanticLabel: 'Theme',
                      controlKey: const ValueKey('theme-preview-scrubber'),
                      dotKeyPrefix: 'theme-pagination-dot-',
                      count: entries.length,
                      selected: currentPage.value.clamp(0, entries.length - 1),
                      animateChanges: animatePaginationChanges,
                      onSelected: (index) {
                        if (index == currentPage.value ||
                            !controller.hasClients) {
                          return;
                        }
                        controller.jumpToPage(index);
                      },
                    ),
                  ),
                  SizedBox(
                    width: 64,
                    child: _AppearanceCycleAction(
                      mode: draftMode.value,
                      reduceMotion: reduceMotion,
                      onTap: cycleAppearance,
                    ),
                  ),
                ],
              ),
            ),
          ),
        ],
      ),
    );
  }
}

class _ThemePreviewPage extends StatelessWidget {
  const _ThemePreviewPage({
    required this.theme,
    required this.mode,
    required this.accentIndex,
    required this.platformBrightness,
    required this.titleHorizontalOffset,
  });

  final ThemeColors theme;
  final ThemeMode mode;
  final int accentIndex;
  final Brightness platformBrightness;
  final double titleHorizontalOffset;

  @override
  Widget build(BuildContext context) => Padding(
    padding: const EdgeInsets.fromLTRB(Grid.gutter, Grid.xxs, Grid.gutter, 0),
    child: DecoratedBox(
      key: ValueKey('theme-preview-page-${theme.name}'),
      decoration: BoxDecoration(
        color: context.colors.surfaceContainerLow,
        borderRadius: BorderRadius.circular(Radii.container),
      ),
      child: LayoutBuilder(
        builder: (context, constraints) {
          const previewWidth = 858.0;
          const previewHeight = 852.0;
          final availableWidth = constraints.maxWidth - Grid.xs * 2;
          final availableHeight = constraints.maxHeight - Grid.xs * 2;
          final widthScale = availableWidth / previewWidth;
          final heightScale = availableHeight / previewHeight;
          final scale = widthScale < heightScale ? widthScale : heightScale;
          final previewTop =
              (constraints.maxHeight - previewHeight * scale) / 2;
          final desiredTitleBottom =
              constraints.maxHeight - previewTop + Grid.md;
          final landscapeTitleBottom =
              constraints.maxHeight - Grid.xs - Grid.lg;
          final titleBottom =
              heightScale < widthScale &&
                  desiredTitleBottom > landscapeTitleBottom
              ? landscapeTitleBottom
              : desiredTitleBottom;

          return Stack(
            children: [
              Positioned.fill(
                child: Padding(
                  padding: const EdgeInsets.all(Grid.xs),
                  child: FittedBox(
                    key: ValueKey('theme-full-preview-${theme.name}'),
                    fit: BoxFit.contain,
                    alignment: Alignment.center,
                    child: _ThemeDevicePairPreview(
                      theme: theme,
                      mode: mode,
                      accentIndex: accentIndex,
                      platformBrightness: platformBrightness,
                    ),
                  ),
                ),
              ),
              Positioned(
                left: Grid.xs,
                right: Grid.xs,
                bottom: titleBottom,
                child: Transform.translate(
                  key: ValueKey('theme-preview-name-motion-${theme.name}'),
                  offset: Offset(titleHorizontalOffset, 0),
                  child: Text(
                    _themeLabelFor(theme, mode),
                    key: ValueKey('theme-preview-name-${theme.name}'),
                    maxLines: 1,
                    overflow: TextOverflow.ellipsis,
                    textAlign: TextAlign.center,
                    style: context.textTheme.titleSmall?.copyWith(
                      fontWeight: FontWeight.w600,
                    ),
                  ),
                ),
              ),
            ],
          );
        },
      ),
    ),
  );
}

class _ThemeDevicePairPreview extends StatelessWidget {
  const _ThemeDevicePairPreview({
    required this.theme,
    required this.mode,
    required this.accentIndex,
    required this.platformBrightness,
  });

  final ThemeColors theme;
  final ThemeMode mode;
  final int accentIndex;
  final Brightness platformBrightness;

  @override
  Widget build(BuildContext context) {
    final displayed = _displayedThemeFor(theme, mode, platformBrightness);
    final resolved = resolveSchemes(theme.name, mode);
    final base = platformBrightness == Brightness.dark
        ? resolved.dark
        : resolved.light;
    final effectiveAccent = isBuzzTheme(displayed.name)
        ? defaultAccentIndex
        : accentIndex;
    final scheme = applyAccent(base, effectiveAccent);
    final gradient = buzzTopSectionGradient(displayed.name, scheme.brightness);
    final previewTheme = scheme.brightness == Brightness.dark
        ? AppTheme.dark(colorScheme: scheme, topSectionGradient: gradient)
        : AppTheme.light(colorScheme: scheme, topSectionGradient: gradient);

    return Row(
      key: ValueKey('theme-device-pair-preview-${theme.name}'),
      mainAxisSize: MainAxisSize.min,
      children: [
        _PreviewDeviceFrame(
          key: ValueKey('theme-full-home-${theme.name}'),
          theme: previewTheme,
          semanticLabel: 'Home preview',
          child: const _FigmaHomeScreen(),
        ),
        const SizedBox(width: 72),
        _PreviewDeviceFrame(
          key: ValueKey('theme-full-chat-${theme.name}'),
          theme: previewTheme,
          semanticLabel: 'Chat preview',
          child: const _FigmaChatScreen(),
        ),
      ],
    );
  }
}

class _ThemePreviewCloseButton extends StatelessWidget {
  const _ThemePreviewCloseButton({required this.onPressed});

  final VoidCallback onPressed;

  @override
  Widget build(BuildContext context) {
    if (defaultTargetPlatform == TargetPlatform.iOS) {
      return IosGlassNavigationButton(
        key: const ValueKey('theme-preview-close'),
        icon: IosGlassNavigationIcon.close,
        semanticLabel: 'Close preview',
        onPressed: onPressed,
        width: 44,
        height: 44,
        foregroundColor: navigationPrimaryForeground(context),
      );
    }
    return SizedBox.square(
      dimension: 44,
      child: IconButton(
        key: const ValueKey('theme-preview-close'),
        tooltip: 'Close preview',
        onPressed: onPressed,
        style: IconButton.styleFrom(
          padding: EdgeInsets.zero,
          backgroundColor: context.colors.surfaceContainerHighest,
          foregroundColor: navigationPrimaryForeground(context),
          shape: RoundedRectangleBorder(
            borderRadius: BorderRadius.circular(Radii.dialog),
          ),
        ),
        icon: const Icon(BuzzIcons.x, size: 22),
      ),
    );
  }
}

class _ThemePreviewSetButton extends StatelessWidget {
  const _ThemePreviewSetButton({required this.onPressed});

  final VoidCallback onPressed;

  @override
  Widget build(BuildContext context) {
    final foreground = navigationPrimaryForeground(context);
    if (defaultTargetPlatform == TargetPlatform.iOS) {
      return IosGlassNavigationAction(
        key: const ValueKey('theme-preview-set'),
        label: 'Set',
        width: 64,
        height: 44,
        foregroundColor: foreground,
        onPressed: onPressed,
      );
    }
    return Material(
      key: const ValueKey('theme-preview-set'),
      color: context.colors.surfaceContainerHighest,
      borderRadius: BorderRadius.circular(Radii.full),
      clipBehavior: Clip.antiAlias,
      child: InkWell(
        onTap: onPressed,
        child: SizedBox(
          height: 44,
          child: Padding(
            padding: const EdgeInsets.symmetric(horizontal: Grid.xs),
            child: Center(
              child: Text(
                'Set',
                style: context.textTheme.labelLarge?.copyWith(
                  color: foreground,
                  fontWeight: FontWeight.w600,
                ),
              ),
            ),
          ),
        ),
      ),
    );
  }
}

class _PreviewSheetCircleAction extends StatelessWidget {
  const _PreviewSheetCircleAction({
    super.key,
    required this.semanticLabel,
    required this.onTap,
    required this.color,
  });

  final String semanticLabel;
  final VoidCallback onTap;
  final Color color;

  @override
  Widget build(BuildContext context) {
    if (defaultTargetPlatform == TargetPlatform.iOS) {
      return IosGlassNavigationButton(
        icon: IosGlassNavigationIcon.colorSwatch,
        semanticLabel: semanticLabel,
        onPressed: onTap,
        width: 64,
        height: 54,
        controlSize: 54,
        buttonCenterX: 32,
        foregroundColor: context.colors.onSurface,
        swatchColor: color,
      );
    }
    return Semantics(
      label: semanticLabel,
      button: true,
      child: InkResponse(
        onTap: onTap,
        radius: 30,
        child: Center(
          child: SizedBox.square(
            dimension: 54,
            child: Container(
              padding: const EdgeInsets.all(4),
              decoration: BoxDecoration(
                color: context.colors.surfaceContainerHighest,
                shape: BoxShape.circle,
                border: Border.all(
                  color: context.colors.outlineVariant,
                  strokeAlign: BorderSide.strokeAlignOutside,
                ),
              ),
              child: DecoratedBox(
                key: const ValueKey('theme-preview-accent-swatch'),
                decoration: BoxDecoration(color: color, shape: BoxShape.circle),
              ),
            ),
          ),
        ),
      ),
    );
  }
}

class _AppearanceCycleAction extends StatelessWidget {
  const _AppearanceCycleAction({
    required this.mode,
    required this.reduceMotion,
    required this.onTap,
  });

  final ThemeMode mode;
  final bool reduceMotion;
  final VoidCallback onTap;

  IconData get _icon => switch (mode) {
    ThemeMode.light => BuzzIcons.sun,
    ThemeMode.dark => BuzzIcons.moon,
    ThemeMode.system => BuzzIcons.sunMoon,
  };

  IosGlassNavigationIcon get _iosIcon => switch (mode) {
    ThemeMode.light => IosGlassNavigationIcon.sun,
    ThemeMode.dark => IosGlassNavigationIcon.moon,
    ThemeMode.system => IosGlassNavigationIcon.systemAppearance,
  };

  @override
  Widget build(BuildContext context) {
    final semanticLabel =
        '${appearanceModeLabel(mode)} appearance. Double tap to change.';
    if (defaultTargetPlatform == TargetPlatform.iOS) {
      return IosGlassNavigationButton(
        key: const ValueKey('theme-preview-appearance-action-button'),
        icon: _iosIcon,
        semanticLabel: semanticLabel,
        onPressed: onTap,
        width: 64,
        height: 54,
        controlSize: 54,
        buttonCenterX: 32,
        foregroundColor: context.colors.onSurface,
      );
    }
    return Semantics(
      label: semanticLabel,
      button: true,
      child: InkResponse(
        key: const ValueKey('theme-preview-appearance-action-button'),
        onTap: onTap,
        radius: 30,
        child: Container(
          width: 54,
          height: 54,
          decoration: BoxDecoration(
            color: context.colors.surfaceContainerHighest,
            shape: BoxShape.circle,
            border: Border.all(color: context.colors.outlineVariant),
          ),
          child: AnimatedSwitcher(
            key: const ValueKey('theme-preview-appearance-switcher'),
            duration: reduceMotion ? Duration.zero : _appearanceMotionDuration,
            switchInCurve: _appearanceMotionCurve,
            switchOutCurve: Curves.easeInOutCubic,
            layoutBuilder: (currentChild, previousChildren) => Stack(
              alignment: Alignment.center,
              children: [...previousChildren, ?currentChild],
            ),
            transitionBuilder: (child, animation) => FadeTransition(
              opacity: animation,
              child: ScaleTransition(
                scale: Tween(begin: 0.82, end: 1.0).animate(animation),
                child: child,
              ),
            ),
            child: Icon(
              _icon,
              key: ValueKey('theme-appearance-${mode.name}'),
              size: 24,
              color: context.colors.onSurface,
            ),
          ),
        ),
      ),
    );
  }
}
