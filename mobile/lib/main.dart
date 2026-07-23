import 'package:flutter/material.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';
import 'package:shared_preferences/shared_preferences.dart';

import 'app.dart';
import 'shared/client/client_headers.dart';
import 'shared/theme/theme_provider.dart';

void main() async {
  WidgetsFlutterBinding.ensureInitialized();

  // Pre-load preferences so the first frame uses the saved theme/accent.
  final (prefs, clientHeaders) = await (
    SharedPreferences.getInstance(),
    loadClientHeaders(),
  ).wait;

  runApp(
    ProviderScope(
      overrides: [
        savedPrefsProvider.overrideWithValue(prefs),
        clientHeadersProvider.overrideWithValue(clientHeaders),
      ],
      child: const App(),
    ),
  );
}
