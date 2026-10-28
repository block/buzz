part of '../theme_picker_page.dart';

const _appearanceOrder = [ThemeMode.light, ThemeMode.dark, ThemeMode.system];
const _appearanceMotionDuration = Duration(milliseconds: 150);
const _appearanceMotionCurve = Cubic(0.23, 1, 0.32, 1);

class _ThemePreviewExperience extends HookConsumerWidget {
  const _ThemePreviewExperience();

  @override
  Widget build(BuildContext context, WidgetRef ref) {
    final stored = ref.read(communityThemeProvider);
    final draftMode = useState(stored.mode);
    final draftAccent = useState(chatColorIndex(stored.accent));
    final platformBrightness = MediaQuery.platformBrightnessOf(context);
    final reduceMotion = MediaQuery.disableAnimationsOf(context);
    final brightness = switch (draftMode.value) {
      ThemeMode.light => Brightness.light,
      ThemeMode.dark => Brightness.dark,
      ThemeMode.system => platformBrightness,
    };
    final selectedEntry = findTheme(
      brightness == Brightness.dark ? buzzDarkThemeName : buzzThemeName,
    )!;
    final selectedBaseScheme = brightness == Brightness.dark
        ? darkColorScheme
        : lightColorScheme;

    void cycleAppearance() {
      final index = _appearanceOrder.indexOf(draftMode.value);
      unawaited(HapticFeedback.selectionClick());
      draftMode.value = _appearanceOrder[(index + 1) % _appearanceOrder.length];
    }

    void applySelection() {
      unawaited(HapticFeedback.mediumImpact());
      ref
          .read(communityThemeProvider.notifier)
          .setPreference(
            CommunityThemePreference(
              theme: draftMode.value == ThemeMode.dark
                  ? buzzDarkThemeName
                  : buzzThemeName,
              accent: chatColorChoices[draftAccent.value].wireValue,
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
            child: _ThemePreviewPage(
              theme: selectedEntry,
              mode: draftMode.value,
              accentIndex: draftAccent.value,
              platformBrightness: platformBrightness,
              titleHorizontalOffset: 0,
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
                    child: _PreviewSheetCircleAction(
                      key: const ValueKey('theme-preview-accent-action-button'),
                      semanticLabel: 'Chat color',
                      color: chatColorForScheme(
                        selectedBaseScheme,
                        draftAccent.value,
                      ),
                      onTap: () => showAccentColorPickerSheet(
                        context: context,
                        colorScheme: selectedBaseScheme,
                        selectedIndex: draftAccent.value,
                        onSelected: (index) => draftAccent.value = index,
                      ),
                    ),
                  ),
                  Expanded(
                    child: Text(
                      appearanceModeLabel(draftMode.value),
                      textAlign: TextAlign.center,
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
    final brightness = switch (mode) {
      ThemeMode.light => Brightness.light,
      ThemeMode.dark => Brightness.dark,
      ThemeMode.system => platformBrightness,
    };
    final previewTheme = mobileAppearanceTheme(
      brightness,
      chatColorChoices[accentIndex].wireValue,
    );

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
        icon: const Icon(BuzzIcons.x, size: 24),
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
