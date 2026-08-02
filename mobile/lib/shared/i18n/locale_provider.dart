import 'package:flutter/material.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';

import '../theme/theme_provider.dart';

const _localeKey = 'buzz.locale';

class LocaleNotifier extends Notifier<Locale?> {
  @override
  Locale? build() {
    final stored = ref.read(savedPrefsProvider).getString(_localeKey);
    return switch (stored) {
      'pt-BR' => const Locale('pt', 'BR'),
      'en-US' => const Locale('en', 'US'),
      _ => null,
    };
  }

  void setLocale(Locale? locale) {
    state = locale;
    final prefs = ref.read(savedPrefsProvider);
    if (locale == null) {
      prefs.remove(_localeKey);
      return;
    }
    prefs.setString(
      _localeKey,
      locale.languageCode == 'pt' ? 'pt-BR' : 'en-US',
    );
  }
}

final localeProvider = NotifierProvider<LocaleNotifier, Locale?>(
  LocaleNotifier.new,
);
