import 'dart:async';

import 'package:flutter/foundation.dart';
import 'package:flutter/material.dart';
import 'package:flutter/services.dart';
import 'package:flutter_hooks/flutter_hooks.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';
import 'package:lucide_icons_flutter/lucide_icons.dart';

import '../../shared/community/community_icon_provider.dart';
import '../../shared/community/community_provider.dart';
import '../../shared/theme/theme.dart';
import '../../shared/widgets/avatar_image.dart';
import '../../shared/widgets/frosted_app_bar.dart';
import '../../shared/widgets/frosted_scaffold.dart';
import '../../shared/widgets/ios_glass_navigation_action.dart';
import '../../shared/widgets/ios_glass_navigation_button.dart';
import '../../shared/widgets/ios_glass_theme_pagination.dart';
import 'theme_option_sheets.dart';

part 'theme_picker_page/theme_home_preview.dart';
part 'theme_picker_page/theme_preview_sheet.dart';

List<ThemeColors> _themeEntriesForMode(ThemeMode mode) {
  final groups = themeGroups();
  return switch (mode) {
    ThemeMode.system => groups.paired,
    ThemeMode.light => groups.light,
    ThemeMode.dark => groups.dark,
  };
}

String _themeLabelFor(ThemeColors theme, ThemeMode mode) =>
    mode == ThemeMode.system ? pairedThemeLabel(theme.name) : theme.displayName;

ThemeColors _displayedThemeFor(
  ThemeColors theme,
  ThemeMode mode,
  Brightness platformBrightness,
) {
  if (mode != ThemeMode.system || platformBrightness == Brightness.light) {
    return theme;
  }
  return findTheme(themePairFor(theme.name) ?? '') ?? theme;
}

bool _themeEntryIsSelected(
  ThemeColors entry,
  ThemeMode mode,
  String selectedScheme,
) {
  final active = effectiveTheme(selectedScheme, mode);
  if (active == null) return false;
  if (mode != ThemeMode.system) return active.name == entry.name;
  return active.name == entry.name || themePairFor(entry.name) == active.name;
}

/// Direct, swipeable theme preview. Theme, accent, and appearance changes stay
/// as a draft until Set is pressed.
class ThemePickerPage extends StatelessWidget {
  const ThemePickerPage({super.key});

  @override
  Widget build(BuildContext context) => const _ThemePreviewExperience();
}
