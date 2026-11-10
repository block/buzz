import 'package:buzz/app.dart';
import 'package:buzz/features/age_gate/age_signal_provider.dart';
import 'package:buzz/shared/auth/auth.dart';
import 'package:buzz/shared/theme/theme_provider.dart';
import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:gpt_markdown/gpt_markdown.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';
import 'package:shared_preferences/shared_preferences.dart';

// The app rebuilds on every lifecycle change, so a resume rebuilds it several
// times. ThemeData has no value equality the app's themes satisfy, so new
// themes made MaterialApp animate between identical ones, and every frame of
// that animation made each mounted message re-parse its markdown.
void main() {
  testWidgets('an app rebuild keeps its themes and markdown theme', (
    tester,
  ) async {
    SharedPreferences.setMockInitialValues({});
    final prefs = await SharedPreferences.getInstance();
    final auth = _SignedOutAuth();
    final container = ProviderContainer(
      overrides: [
        savedPrefsProvider.overrideWithValue(prefs),
        authProvider.overrideWith(() => auth),
        ageSignalProvider.overrideWith(_AllowedAgeSignal.new),
      ],
    );
    addTearDown(container.dispose);
    await tester.pumpWidget(
      UncontrolledProviderScope(container: container, child: const App()),
    );
    await tester.pump(const Duration(seconds: 1));

    MaterialApp app() => tester.widget<MaterialApp>(find.byType(MaterialApp));
    GptMarkdownThemeData markdown() => tester
        .widget<GptMarkdownTheme>(find.byType(GptMarkdownTheme).first)
        .gptThemeData;
    final theme = app().theme;
    final darkTheme = app().darkTheme;
    final markdownTheme = markdown();

    // A new auth state rebuilds the app, as a lifecycle change does.
    auth.republish();
    await tester.pump();
    // Midway through what would be a theme animation.
    await tester.pump(const Duration(milliseconds: 100));

    expect(identical(app().theme, theme), isTrue);
    expect(identical(app().darkTheme, darkTheme), isTrue);
    expect(identical(markdown(), markdownTheme), isTrue);

    await tester.pumpWidget(const SizedBox.shrink());
  });
}

class _SignedOutAuth extends AuthNotifier {
  @override
  Future<AuthState> build() async =>
      const AuthState(status: AuthStatus.unauthenticated);

  void republish() =>
      state = AsyncData(AuthState(status: state.requireValue.status));
}

class _AllowedAgeSignal extends AgeSignalNotifier {
  @override
  AgeSignalState build() => AgeSignalState.allowed;

  @override
  Future<void> request() async {}
}
