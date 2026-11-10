import 'package:flutter/material.dart';

import 'app_colors.dart';

/// Shared neutral background for mobile controls and selected tabs.
Color neutralControlSurface(Brightness brightness) =>
    brightness == Brightness.dark
    ? const Color(0xFF101010)
    : const Color(0xFFF0F0F0);

extension AppThemeExtension on BuildContext {
  ThemeData get theme => Theme.of(this);
  ColorScheme get colors => theme.colorScheme;
  Color get controlSurface => neutralControlSurface(theme.brightness);

  /// Design-system text-tertiary (neutral-9), for quiet author labels.
  Color get tertiaryTextColor => theme.brightness == Brightness.dark
      ? const Color(0xFFA6A6A6)
      : const Color(0xFF8D8D8D);
  TextTheme get textTheme => theme.textTheme;

  /// Design-system availability color: success, warning, or the muted
  /// text-tertiary foreground. `outline` is a hairline-border token and reads
  /// as invisible on light surfaces, so it must not be used for a dot.
  Color presenceColor(String? presence) => switch (presence) {
    'online' => appColors.success,
    'away' => appColors.warning,
    _ => colors.onSurfaceVariant,
  };

  AppColors get appColors {
    final ext = theme.extension<AppColors>();
    assert(ext != null, 'AppColors not found in ThemeData.extensions');
    return ext!;
  }
}
