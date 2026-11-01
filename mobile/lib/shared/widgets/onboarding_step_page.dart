import 'dart:ui' show ImageFilter;
import 'package:flutter/material.dart';
import 'package:flutter/foundation.dart';
import 'package:flutter/semantics.dart';

import '../theme/theme.dart';
import 'ios_navigation_bar.dart';
import 'navigation_backdrop.dart';

/// Opaque onboarding chrome shared by account setup and identity import.
/// A scrollable minimum-height body keeps actions at the bottom without
/// clipping content when the keyboard or larger accessibility text is shown.
class OnboardingStepPage extends StatelessWidget {
  const OnboardingStepPage({
    super.key,
    required this.title,
    this.subtitle,
    this.onConnectionOptions,
    this.foregroundProgress = 0,
    this.keyboardBottomSpacing = Grid.sm,
    required this.onBack,
    required this.content,
    this.actions = const [],
  });

  final String title;
  final String? subtitle;
  final VoidCallback? onConnectionOptions;
  final double foregroundProgress;
  final double keyboardBottomSpacing;
  final VoidCallback? onBack;
  final Widget content;
  final List<Widget> actions;

  @override
  Widget build(BuildContext context) {
    final theme = Theme.of(context);
    final ios = defaultTargetPlatform == TargetPlatform.iOS;
    final buttonStyle = FilledButton.styleFrom(
      backgroundColor: theme.colorScheme.primary,
      foregroundColor: theme.colorScheme.onPrimary,
      minimumSize: const Size.fromHeight(52),
      shape: const StadiumBorder(),
    );
    return Theme(
      data: theme.copyWith(
        filledButtonTheme: FilledButtonThemeData(style: buttonStyle),
        textButtonTheme: TextButtonThemeData(
          style: TextButton.styleFrom(
            foregroundColor: theme.colorScheme.onSurface,
            minimumSize: const Size.fromHeight(52),
            shape: const StadiumBorder(),
          ),
        ),
      ),
      child: PopScope(
        canPop: false,
        onPopInvokedWithResult: (didPop, _) {
          if (!didPop) onBack?.call();
        },
        child: Scaffold(
          backgroundColor: theme.colorScheme.surface,
          body: SafeArea(
            top: !ios,
            child: ExcludeFocus(
              excluding: foregroundProgress > 0,
              child: ExcludeSemantics(
                excluding: foregroundProgress > 0,
                child: IgnorePointer(
                  ignoring: foregroundProgress > 0,
                  child: Opacity(
                    key: const Key('scanner-welcome-opacity'),
                    opacity: 1 - foregroundProgress,
                    child: Transform.scale(
                      key: const Key('scanner-welcome-scale'),
                      scale: MediaQuery.disableAnimationsOf(context)
                          ? 1
                          : 1 - .04 * foregroundProgress,
                      child: ImageFiltered(
                        enabled:
                            !MediaQuery.disableAnimationsOf(context) &&
                            foregroundProgress > 0 &&
                            foregroundProgress < 1,
                        imageFilter: ImageFilter.blur(
                          sigmaX: 16 * foregroundProgress,
                          sigmaY: 16 * foregroundProgress,
                        ),
                        child: Column(
                          crossAxisAlignment: CrossAxisAlignment.stretch,
                          children: [
                            if (ios)
                              SizedBox(
                                height:
                                    MediaQuery.paddingOf(context).top +
                                    IosNavigationMetrics.of(
                                      context,
                                    ).compactHeight,
                                child: IosNavigationBar(
                                  title: '',
                                  leading: onBack == null
                                      ? null
                                      : IosNavigationAction(
                                          label: 'Back',
                                          symbol: 'chevron.backward',
                                          onPressed: onBack,
                                        ),
                                ),
                              )
                            else if (onBack != null)
                              NavigationBackdrop(
                                child: Align(
                                  alignment: AlignmentDirectional.centerStart,
                                  child: Padding(
                                    padding: const EdgeInsets.symmetric(
                                      horizontal: Grid.xxs,
                                    ),
                                    child: IconButton(
                                      tooltip: 'Back',
                                      onPressed: onBack,
                                      icon: const BackButtonIcon(),
                                    ),
                                  ),
                                ),
                              ),
                            Expanded(
                              child: CustomScrollView(
                                keyboardDismissBehavior:
                                    ScrollViewKeyboardDismissBehavior.onDrag,
                                slivers: [
                                  SliverFillRemaining(
                                    hasScrollBody: false,
                                    child: Padding(
                                      padding: EdgeInsets.fromLTRB(
                                        Grid.sm,
                                        Grid.sm,
                                        Grid.sm,
                                        MediaQuery.viewInsetsOf(
                                                  context,
                                                ).bottom >
                                                0
                                            ? keyboardBottomSpacing
                                            : Grid.sm,
                                      ),
                                      child: Column(
                                        crossAxisAlignment:
                                            CrossAxisAlignment.stretch,
                                        children: [
                                          GestureDetector(
                                            key: onConnectionOptions == null
                                                ? null
                                                : const Key(
                                                    'developer-pairing-trigger',
                                                  ),
                                            excludeFromSemantics: true,
                                            child: Semantics(
                                              header: true,
                                              namesRoute: true,
                                              customSemanticsActions:
                                                  onConnectionOptions == null
                                                  ? null
                                                  : {
                                                      const CustomSemanticsAction(
                                                        label:
                                                            'More connection options',
                                                      ): onConnectionOptions!,
                                                    },
                                              child: Text(
                                                title,
                                                style: OnboardingTypography
                                                    .title
                                                    .copyWith(
                                                      color: theme
                                                          .colorScheme
                                                          .onSurface,
                                                    ),
                                              ),
                                            ),
                                          ),
                                          if (subtitle != null) ...[
                                            const SizedBox(height: Grid.xs),
                                            Text(
                                              subtitle!,
                                              style: OnboardingTypography
                                                  .subtitle
                                                  .copyWith(
                                                    color: theme
                                                        .colorScheme
                                                        .onSurfaceVariant,
                                                  ),
                                            ),
                                          ],
                                          const SizedBox(height: Grid.xs),
                                          content,
                                          const Spacer(),
                                          if (actions.isNotEmpty)
                                            const SizedBox(height: Grid.sm),
                                          for (
                                            var i = 0;
                                            i < actions.length;
                                            i++
                                          ) ...[
                                            if (i > 0)
                                              const SizedBox(height: Grid.xxs),
                                            actions[i],
                                          ],
                                        ],
                                      ),
                                    ),
                                  ),
                                ],
                              ),
                            ),
                          ],
                        ),
                      ),
                    ),
                  ),
                ),
              ),
            ),
          ),
        ),
      ),
    );
  }
}
