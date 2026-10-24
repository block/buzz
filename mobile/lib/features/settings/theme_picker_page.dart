import 'dart:async';

import 'package:flutter/foundation.dart';
import 'package:flutter/material.dart';
import 'package:flutter/services.dart';
import 'package:flutter_hooks/flutter_hooks.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';
import 'package:buzz/shared/theme/buzz_icons.dart';

import '../../shared/community/community_icon_provider.dart';
import '../../shared/community/community_provider.dart';
import '../../shared/theme/theme.dart';
import '../../shared/widgets/avatar_image.dart';
import '../../shared/widgets/frosted_app_bar.dart';
import '../../shared/widgets/frosted_scaffold.dart';
import '../../shared/widgets/ios_glass_navigation_action.dart';
import '../../shared/widgets/ios_glass_navigation_button.dart';
import 'theme_option_sheets.dart';

part 'theme_picker_page/theme_home_preview.dart';
part 'theme_picker_page/theme_preview_sheet.dart';

String _themeLabelFor(ThemeColors theme, ThemeMode mode) =>
    mode == ThemeMode.system ? pairedThemeLabel(theme.name) : theme.displayName;

/// Appearance preview with draft mode and chat color, applied with Set.
class ThemePickerPage extends StatelessWidget {
  const ThemePickerPage({super.key});

  @override
  Widget build(BuildContext context) => const _ThemePreviewExperience();
}
