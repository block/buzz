import 'package:flutter/material.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';

import '../shared/theme/app_theme.dart';
import 'share_sheet.dart';

/// Entry point the iOS share extension runs in its own Flutter engine. It uses
/// no plugins: everything native goes through the `buzz/share_extension`
/// channel.
@pragma('vm:entry-point')
void shareExtensionMain() {
  WidgetsFlutterBinding.ensureInitialized();
  runApp(
    ProviderScope(
      child: MaterialApp(
        debugShowCheckedModeBanner: false,
        theme: AppTheme.light(),
        darkTheme: AppTheme.dark(),
        home: const ShareSheet(),
      ),
    ),
  );
}
