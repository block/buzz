import 'package:flutter/material.dart';
import 'package:flutter/services.dart';

import 'app_colors.dart';
import 'color_scheme.dart';
import 'grid.dart';
import 'text_theme.dart';

/// Border radius constants matching desktop shadcn "New York" style.
/// Desktop uses --radius: 0.625rem (10px) as base:
///   lg = 10px, md = 8px, sm = 6px
class Radii {
  static const double lg = 10.0;
  static const double md = 8.0;
  static const double sm = 6.0;
  static const double dialog = 24.0; // desktop uses rounded-3xl for dialogs
}

class AppTheme {
  static ThemeData light({ColorScheme? colorScheme}) {
    final scheme = colorScheme ?? lightColorScheme;
    final appColors = AppColors(
      success: const Color(0xFF40A02B), // Catppuccin Latte Green — universal
      warning: const Color(0xFFDF8E1D), // Latte Yellow
      accent: scheme.tertiary,
    );

    return _buildTheme(
      scheme: scheme,
      appColors: appColors,
      brightness: Brightness.light,
      statusBarIconBrightness: Brightness.dark,
      statusBarBrightness: Brightness.light,
    );
  }

  static ThemeData dark({ColorScheme? colorScheme}) {
    final scheme = colorScheme ?? darkColorScheme;
    final appColors = AppColors(
      success: const Color(
        0xFFA6DA95,
      ), // Catppuccin Macchiato Green — universal
      warning: const Color(0xFFEED49F), // Macchiato Yellow
      accent: scheme.tertiary,
    );

    return _buildTheme(
      scheme: scheme,
      appColors: appColors,
      brightness: Brightness.dark,
      statusBarIconBrightness: Brightness.light,
      statusBarBrightness: Brightness.dark,
    );
  }

  static ThemeData _buildTheme({
    required ColorScheme scheme,
    required AppColors appColors,
    required Brightness brightness,
    required Brightness statusBarIconBrightness,
    required Brightness statusBarBrightness,
  }) {
    return ThemeData(
      useMaterial3: true,
      colorScheme: scheme,
      extensions: [appColors],
      fontFamily: 'Geist',
      textTheme: textTheme,
      appBarTheme: AppBarTheme(
        backgroundColor: Colors.transparent,
        foregroundColor: scheme.onSurface,
        surfaceTintColor: Colors.transparent,
        elevation: 0,
        scrolledUnderElevation: 0,
        titleTextStyle: textTheme.titleMedium?.copyWith(
          color: scheme.onSurface,
        ),
        systemOverlayStyle: SystemUiOverlayStyle(
          statusBarColor: Colors.transparent,
          statusBarIconBrightness: statusBarIconBrightness,
          statusBarBrightness: statusBarBrightness,
        ),
      ),

      // Bottom navigation: clean style, no indicator pill
      navigationBarTheme: NavigationBarThemeData(
        backgroundColor: scheme.surface,
        elevation: 0,
        indicatorColor: Colors.transparent,
        iconTheme: WidgetStateProperty.resolveWith((states) {
          if (states.contains(WidgetState.selected)) {
            return IconThemeData(color: scheme.primary, size: 24);
          }
          return IconThemeData(color: scheme.onSurfaceVariant, size: 24);
        }),
        labelTextStyle: WidgetStateProperty.resolveWith((states) {
          if (states.contains(WidgetState.selected)) {
            return textTheme.labelSmall?.copyWith(
              color: scheme.primary,
              fontWeight: FontWeight.w600,
            );
          }
          return textTheme.labelSmall?.copyWith(color: scheme.onSurfaceVariant);
        }),
      ),

      // Buttons: desktop uses rounded-md (8px), h-9 (36px), px-4 (16px)
      elevatedButtonTheme: ElevatedButtonThemeData(
        style: ElevatedButton.styleFrom(
          backgroundColor: scheme.primary,
          foregroundColor: scheme.onPrimary,
          elevation: 0,
          padding: const EdgeInsets.symmetric(horizontal: 16, vertical: 10),
          minimumSize: const Size(0, 36),
          shape: RoundedRectangleBorder(
            borderRadius: BorderRadius.circular(Radii.md),
          ),
          textStyle: textTheme.labelMedium?.copyWith(
            fontWeight: FontWeight.w500,
          ),
        ),
      ),
      filledButtonTheme: FilledButtonThemeData(
        style: FilledButton.styleFrom(
          elevation: 0,
          padding: const EdgeInsets.symmetric(horizontal: 16, vertical: 10),
          minimumSize: const Size(0, 36),
          shape: RoundedRectangleBorder(
            borderRadius: BorderRadius.circular(Radii.md),
          ),
          textStyle: textTheme.labelMedium?.copyWith(
            fontWeight: FontWeight.w500,
          ),
        ),
      ),
      outlinedButtonTheme: OutlinedButtonThemeData(
        style: OutlinedButton.styleFrom(
          backgroundColor: scheme.surface,
          foregroundColor: scheme.onSurface,
          side: BorderSide(color: scheme.outline, width: 1),
          padding: const EdgeInsets.symmetric(horizontal: 16, vertical: 10),
          minimumSize: const Size(0, 36),
          shape: RoundedRectangleBorder(
            borderRadius: BorderRadius.circular(Radii.md),
          ),
          textStyle: textTheme.labelMedium?.copyWith(
            fontWeight: FontWeight.w500,
          ),
        ),
      ),
      textButtonTheme: TextButtonThemeData(
        style: TextButton.styleFrom(
          foregroundColor: scheme.onSurface,
          padding: const EdgeInsets.symmetric(horizontal: 16, vertical: 8),
          minimumSize: const Size(0, 36),
          shape: RoundedRectangleBorder(
            borderRadius: BorderRadius.circular(Radii.md),
          ),
          textStyle: textTheme.labelMedium?.copyWith(
            fontWeight: FontWeight.w500,
          ),
        ),
      ),

      // Cards: desktop uses rounded-lg (10px), flat, no elevation
      cardTheme: CardThemeData(
        color: scheme.surfaceContainerHighest,
        margin: EdgeInsets.zero,
        elevation: 0,
        shape: RoundedRectangleBorder(
          borderRadius: BorderRadius.circular(Radii.lg),
        ),
      ),

      // Inputs: desktop uses outlined style, rounded-md (8px), h-9 (36px)
      inputDecorationTheme: InputDecorationTheme(
        filled: false,
        border: OutlineInputBorder(
          borderRadius: BorderRadius.circular(Radii.md),
          borderSide: BorderSide(color: scheme.outline),
        ),
        enabledBorder: OutlineInputBorder(
          borderRadius: BorderRadius.circular(Radii.md),
          borderSide: BorderSide(color: scheme.outline),
        ),
        focusedBorder: OutlineInputBorder(
          borderRadius: BorderRadius.circular(Radii.md),
          borderSide: BorderSide(color: scheme.primary),
        ),
        errorBorder: OutlineInputBorder(
          borderRadius: BorderRadius.circular(Radii.md),
          borderSide: BorderSide(color: scheme.error),
        ),
        focusedErrorBorder: OutlineInputBorder(
          borderRadius: BorderRadius.circular(Radii.md),
          borderSide: BorderSide(color: scheme.error),
        ),
        contentPadding: const EdgeInsets.symmetric(
          horizontal: 12,
          vertical: 10,
        ),
        isDense: true,
      ),

      // Dialogs: desktop uses rounded-3xl (24px), custom overlay
      dialogTheme: DialogThemeData(
        backgroundColor: scheme.surface,
        elevation: 0,
        shape: RoundedRectangleBorder(
          borderRadius: BorderRadius.circular(Radii.dialog),
          side: BorderSide(color: scheme.outline),
        ),
        titleTextStyle: textTheme.titleLarge?.copyWith(
          color: scheme.onSurface,
          fontSize: 18,
          fontWeight: FontWeight.w600,
          letterSpacing: -0.3,
        ),
        contentTextStyle: textTheme.bodyMedium?.copyWith(
          color: scheme.onSurfaceVariant,
        ),
      ),

      progressIndicatorTheme: ProgressIndicatorThemeData(
        strokeWidth: 2,
        color: scheme.primary,
        circularTrackColor: scheme.onSurfaceVariant.withValues(alpha: 0.2),
      ),

      listTileTheme: ListTileThemeData(
        titleTextStyle: textTheme.titleSmall?.copyWith(color: scheme.onSurface),
        subtitleTextStyle: textTheme.bodyMedium?.copyWith(
          color: scheme.secondary,
        ),
        iconColor: scheme.secondary,
        contentPadding: const EdgeInsets.symmetric(horizontal: Grid.twelve),
        minVerticalPadding: Grid.twelve,
        horizontalTitleGap: Grid.twelve,
        shape: RoundedRectangleBorder(
          borderRadius: BorderRadius.circular(Radii.md),
        ),
      ),

      // Chips: desktop uses rounded-sm (6px)
      chipTheme: ChipThemeData(
        labelStyle: textTheme.bodySmall?.copyWith(color: scheme.secondary),
        // M3 resolves the chip container via `color` (WidgetStateProperty);
        // `selectedColor` is the legacy M2 path and is ignored here. Selected
        // filter chips (Pulse/Search/Activity tabs) use the accent.
        color: WidgetStateProperty.resolveWith((states) {
          if (states.contains(WidgetState.selected)) return scheme.primary;
          return scheme.surfaceContainerHighest;
        }),
        checkmarkColor: scheme.onPrimary,
        shape: RoundedRectangleBorder(
          side: BorderSide.none,
          borderRadius: BorderRadius.circular(Radii.sm),
        ),
        side: BorderSide.none,
        padding: const EdgeInsets.symmetric(horizontal: 8),
        labelPadding: EdgeInsets.zero,
      ),

      // Popups/menus: desktop uses rounded-md (8px)
      popupMenuTheme: PopupMenuThemeData(
        color: scheme.surface,
        elevation: 4,
        shape: RoundedRectangleBorder(
          borderRadius: BorderRadius.circular(Radii.md),
          side: BorderSide(color: scheme.outline),
        ),
      ),

      // Bottom sheet: match dialog radius
      bottomSheetTheme: BottomSheetThemeData(
        backgroundColor: scheme.surface,
        elevation: 0,
        shape: const RoundedRectangleBorder(
          borderRadius: BorderRadius.vertical(
            top: Radius.circular(Radii.dialog),
          ),
        ),
      ),

      // Tooltips: desktop uses rounded-md, primary bg
      tooltipTheme: TooltipThemeData(
        decoration: BoxDecoration(
          color: scheme.primary,
          borderRadius: BorderRadius.circular(Radii.md),
        ),
        textStyle: textTheme.bodySmall?.copyWith(
          color: scheme.onPrimary,
          fontSize: 12,
        ),
        padding: const EdgeInsets.symmetric(horizontal: 12, vertical: 6),
      ),

      dividerTheme: DividerThemeData(
        color: scheme.outline,
        thickness: 1,
        space: 1,
      ),

      // Snackbar
      snackBarTheme: SnackBarThemeData(
        behavior: SnackBarBehavior.floating,
        shape: RoundedRectangleBorder(
          borderRadius: BorderRadius.circular(Radii.md),
        ),
      ),
    );
  }
}
