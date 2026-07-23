import 'package:flutter/material.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';
import 'package:shared_preferences/shared_preferences.dart';

import 'app.dart';
import 'shared/client/client_headers.dart';
import 'shared/theme/theme_provider.dart';

void main() async {
  WidgetsFlutterBinding.ensureInitialized();

  // Pre-load preferences so the first frame uses the saved theme/accent.
  final prefs = await SharedPreferences.getInstance();
  var clientHeaders = ClientHeaders.empty;
  try {
    clientHeaders = await loadClientHeaders();
  } catch (error) {
    debugPrint('Could not load optional Buzz client headers: $error');
  }

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
