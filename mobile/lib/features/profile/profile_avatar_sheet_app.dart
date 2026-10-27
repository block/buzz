import 'package:flutter/material.dart';
import 'package:flutter/services.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';
import 'package:shared_preferences/shared_preferences.dart';

import '../../shared/theme/app_theme.dart';
import '../../shared/widgets/ios_navigation_metrics.dart';
import '../../shared/theme/theme_provider.dart';
import 'profile_avatar_draft.dart';
import 'profile_setup_avatar_editor.dart';

/// A local-only editor hosted inside the existing UIKit sheet's navigation stack.
@pragma('vm:entry-point')
Future<void> profileAvatarEditorMain() async {
  WidgetsFlutterBinding.ensureInitialized();
  const channel = MethodChannel('buzz/profile_avatar_sheet');
  final initial =
      await channel.invokeMapMethod<String, dynamic>('initial') ?? {};
  final prefs = await SharedPreferences.getInstance();
  runApp(
    ProviderScope(
      overrides: [savedPrefsProvider.overrideWithValue(prefs)],
      child: MaterialApp(
        debugShowCheckedModeBanner: false,
        builder: (context, child) =>
            IosNavigationMetricsHost(child: child ?? const SizedBox.shrink()),
        theme: AppTheme.light(),
        darkTheme: AppTheme.dark(),
        themeMode: initial['dark'] == true ? ThemeMode.dark : ThemeMode.light,
        home: ProfileSetupAvatarEditor(
          anchoredPreview: initial['previewRect'] is Map
              ? Rect.fromLTWH(
                  (initial['previewRect']['x'] as num).toDouble(),
                  (initial['previewRect']['y'] as num).toDouble(),
                  (initial['previewRect']['width'] as num).toDouble(),
                  (initial['previewRect']['height'] as num).toDouble(),
                )
              : null,
          initialDraft: decodeProfileAvatarDraft(initial['draft'] as Map?),
          onPreviewReady: (rect) => channel.invokeMethod<void>('ready', {
            'x': rect.left,
            'y': rect.top,
            'width': rect.width,
            'height': rect.height,
          }),
          onFinished: (draft) => channel.invokeMethod<void>(
            'finish',
            encodeProfileAvatarDraft(draft),
          ),
        ),
      ),
    ),
  );
}
