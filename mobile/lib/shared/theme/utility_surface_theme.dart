import 'package:flutter/material.dart';

import 'app_theme.dart';

/// Returns a theme with the page and container surface roles exchanged.
///
/// The marker makes the transform idempotent: a sheet opened from an already
/// transformed utility page keeps the same hierarchy instead of swapping the
/// two colors back again.
ThemeData utilitySurfaceThemeData(ThemeData theme) {
  if (theme.extension<_UtilitySurfaceThemeMarker>() != null) return theme;

  final colors = theme.colorScheme;
  final pageSurface = colors.surfaceContainerHighest;
  final containerSurface = colors.surface;
  final containerShape = RoundedRectangleBorder(
    borderRadius: BorderRadius.circular(Radii.container),
  );

  return theme.copyWith(
    colorScheme: colors.copyWith(
      surface: pageSurface,
      surfaceContainerHighest: containerSurface,
    ),
    scaffoldBackgroundColor: pageSurface,
    cardTheme: theme.cardTheme.copyWith(
      color: containerSurface,
      shape: containerShape,
    ),
    inputDecorationTheme: theme.inputDecorationTheme.copyWith(
      border: _withUtilityContainerRadius(theme.inputDecorationTheme.border),
      enabledBorder: _withUtilityContainerRadius(
        theme.inputDecorationTheme.enabledBorder,
      ),
      focusedBorder: _withUtilityContainerRadius(
        theme.inputDecorationTheme.focusedBorder,
      ),
      errorBorder: _withUtilityContainerRadius(
        theme.inputDecorationTheme.errorBorder,
      ),
      focusedErrorBorder: _withUtilityContainerRadius(
        theme.inputDecorationTheme.focusedErrorBorder,
      ),
      disabledBorder: _withUtilityContainerRadius(
        theme.inputDecorationTheme.disabledBorder,
      ),
    ),
    bottomSheetTheme: theme.bottomSheetTheme.copyWith(
      backgroundColor: pageSurface,
      modalBackgroundColor: pageSurface,
    ),
    extensions: [
      ...theme.extensions.values,
      const _UtilitySurfaceThemeMarker(),
    ],
  );
}

InputBorder? _withUtilityContainerRadius(InputBorder? border) {
  if (border is! OutlineInputBorder) return border;
  return border.copyWith(borderRadius: BorderRadius.circular(Radii.container));
}

@immutable
class _UtilitySurfaceThemeMarker
    extends ThemeExtension<_UtilitySurfaceThemeMarker> {
  const _UtilitySurfaceThemeMarker();

  @override
  _UtilitySurfaceThemeMarker copyWith() => this;

  @override
  _UtilitySurfaceThemeMarker lerp(
    covariant ThemeExtension<_UtilitySurfaceThemeMarker>? other,
    double t,
  ) => this;
}
