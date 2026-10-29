import 'dart:math' show max;

import 'package:flutter/material.dart';

import '../theme/theme.dart';
import 'bee_loading_mark.dart';

/// Centers the loading mark and heading together as one block.
class OnboardingLoadingPage extends StatelessWidget {
  const OnboardingLoadingPage({super.key, required this.title});
  final String title;

  @override
  Widget build(BuildContext context) {
    final theme = utilitySurfaceThemeData(Theme.of(context));
    final safe = MediaQuery.paddingOf(context);
    return PopScope(
      canPop: false,
      child: Theme(
        data: theme,
        child: Scaffold(
          backgroundColor: theme.colorScheme.surface,
          body: LayoutBuilder(
            builder: (context, bounds) => SingleChildScrollView(
              child: ConstrainedBox(
                constraints: BoxConstraints(minHeight: bounds.maxHeight),
                child: Padding(
                  padding: EdgeInsets.symmetric(
                    horizontal: Grid.sm,
                    vertical: max(safe.top, safe.bottom) + Grid.sm,
                  ),
                  child: Column(
                    mainAxisSize: MainAxisSize.min,
                    mainAxisAlignment: MainAxisAlignment.center,
                    crossAxisAlignment: CrossAxisAlignment.stretch,
                    children: [
                      const Center(child: BeeLoadingMark()),
                      const SizedBox(height: Grid.sm),
                      Semantics(
                        header: true,
                        namesRoute: true,
                        child: Text(
                          title,
                          textAlign: TextAlign.center,
                          style: OnboardingTypography.title.copyWith(
                            color: theme.colorScheme.onSurface,
                          ),
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
    );
  }
}
