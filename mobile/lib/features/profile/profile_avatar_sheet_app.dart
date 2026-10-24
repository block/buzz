import 'package:flutter/material.dart';
import 'package:flutter/services.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';
import 'package:shared_preferences/shared_preferences.dart';

import '../../shared/theme/app_theme.dart';
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
        theme: AppTheme.light(),
        darkTheme: AppTheme.dark(),
        themeMode: initial['dark'] == true ? ThemeMode.dark : ThemeMode.light,
        home: ProfileSetupAvatarEditor(
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
