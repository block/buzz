import 'dart:async';

import 'package:flutter/foundation.dart';
import 'package:duo_kit/duo_kit.dart';
import 'package:flutter/material.dart';
import 'package:flutter/services.dart';
import 'package:flutter_hooks/flutter_hooks.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';
import 'package:buzz/shared/theme/buzz_icons.dart';

import '../../shared/community/community_icon_provider.dart';
import '../../shared/community/community_provider.dart';
import '../../shared/theme/theme.dart';
import '../../shared/widgets/avatar_image.dart';
import '../../shared/widgets/duo_toolbar.dart';
import '../../shared/widgets/ios_navigation_metrics.dart';
import '../../shared/widgets/frosted_app_bar.dart';
import '../../shared/widgets/frosted_scaffold.dart';
import '../../shared/widgets/ios_glass_navigation_action.dart';
import '../../shared/widgets/ios_glass_navigation_button.dart';
import 'theme_option_sheets.dart';

part 'theme_picker_page/theme_home_preview.dart';
part 'theme_picker_page/theme_preview_sheet.dart';

/// Appearance preview with draft mode and chat color, applied with Set.
class ThemePickerPage extends StatelessWidget {
  const ThemePickerPage({super.key});

  @override
  Widget build(BuildContext context) {
    final parent = DuoToolbarScope.maybeOf(context);
    final strip = parent == null
        ? FoldGeometry.of(context).strip
        : parent.strip;
    final insets =
        parent?.contentInsets ??
        EdgeInsets.only(
          left: strip?.side == FoldStripSide.left ? strip!.width : 0,
          right: strip?.side == FoldStripSide.right ? strip!.width : 0,
        );
    return DuoToolbarScope(
      strip: strip,
      active: parent?.active ?? true,
      contentInsets: insets,
      child: MediaQuery.removePadding(
        context: context,
        removeLeft: insets.left > 0,
        removeRight: insets.right > 0,
        child: const _ThemePreviewExperience(),
      ),
    );
  }
}
