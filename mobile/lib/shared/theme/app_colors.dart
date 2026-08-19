import 'package:flutter/material.dart';

@immutable
class AppColors extends ThemeExtension<AppColors> {
  final Color success;
  final Color warning;
  final Color accent;

  /// Gradient for the app's top section, non-null only under the Buzz themes.
  /// Carried on the theme rather than read from a provider so any surface can
  /// opt in via `context.appColors.topSectionGradient` — see
  /// `buzzTopSectionGradient`.
  final Gradient? topSectionGradient;

  const AppColors({
    required this.success,
    required this.warning,
    required this.accent,
    this.topSectionGradient,
  });

  @override
  AppColors copyWith({
    Color? success,
    Color? warning,
    Color? accent,
    Gradient? topSectionGradient,
  }) => AppColors(
    success: success ?? this.success,
    warning: warning ?? this.warning,
    accent: accent ?? this.accent,
    topSectionGradient: topSectionGradient ?? this.topSectionGradient,
  );

  @override
  AppColors lerp(ThemeExtension<AppColors>? other, double t) {
    if (other is! AppColors) return this;
    return AppColors(
      success: Color.lerp(success, other.success, t)!,
      warning: Color.lerp(warning, other.warning, t)!,
      accent: Color.lerp(accent, other.accent, t)!,
      topSectionGradient: Gradient.lerp(
        topSectionGradient,
        other.topSectionGradient,
        t,
      ),
    );
  }
}
