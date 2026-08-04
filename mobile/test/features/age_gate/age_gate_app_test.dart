import 'dart:async';

import 'package:buzz/app.dart';
import 'package:buzz/features/age_gate/age_restriction_page.dart';
import 'package:buzz/features/age_gate/age_signal_provider.dart';
import 'package:buzz/features/home/home_page.dart';
import 'package:buzz/shared/auth/auth.dart';
import 'package:buzz/shared/theme/theme_provider.dart';
import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';
import 'package:shared_preferences/shared_preferences.dart';

void main() {
  TestWidgetsFlutterBinding.ensureInitialized();

  tearDown(() {
    TestDefaultBinaryMessengerBinding.instance.defaultBinaryMessenger
        .setMockMethodCallHandler(ageSignalChannel, null);
  });

  testWidgets('blocks authenticated app content', (tester) async {
    SharedPreferences.setMockInitialValues({});
    final prefs = await SharedPreferences.getInstance();

    await tester.pumpWidget(
      ProviderScope(
        overrides: [
          authProvider.overrideWith(() => _AuthenticatedAuthNotifier()),
          ageSignalProvider.overrideWith(() => _BlockingAgeSignalNotifier()),
          savedPrefsProvider.overrideWithValue(prefs),
        ],
        child: const App(),
      ),
    );
    await tester.pump();

    expect(find.byType(AgeRestrictionPage), findsOneWidget);
    expect(find.byType(HomePage), findsNothing);
  });

  testWidgets('requests after the first frame and gates pushed routes', (
    tester,
  ) async {
    final response = Completer<Object?>();
    var requests = 0;
    TestDefaultBinaryMessengerBinding.instance.defaultBinaryMessenger
        .setMockMethodCallHandler(ageSignalChannel, (call) {
          requests += 1;
          return response.future;
        });
    SharedPreferences.setMockInitialValues({});
    final prefs = await SharedPreferences.getInstance();

    await tester.pumpWidget(
      ProviderScope(
        overrides: [
          authProvider.overrideWith(() => _FakeAuthNotifier()),
          savedPrefsProvider.overrideWithValue(prefs),
        ],
        child: const App(),
      ),
    );

    expect(requests, 1);
    final navigator = tester.state<NavigatorState>(find.byType(Navigator));
    unawaited(
      navigator.push<void>(
        MaterialPageRoute<void>(
          builder: (_) => const Scaffold(body: Text('Pushed route')),
        ),
      ),
    );
    await tester.pumpAndSettle();
    expect(find.text('Pushed route'), findsOneWidget);

    response.complete({'status': 'signal', 'ageUpper': 17});
    await tester.pump();
    await tester.pump();

    expect(find.byType(AgeRestrictionPage), findsOneWidget);
    expect(find.text('Pushed route'), findsNothing);
    expect(requests, 1);
  });
}

class _AuthenticatedAuthNotifier extends AuthNotifier {
  @override
  Future<AuthState> build() async {
    return const AuthState(status: AuthStatus.authenticated);
  }
}

class _BlockingAgeSignalNotifier extends AgeSignalNotifier {
  @override
  bool build() => true;
}

class _FakeAuthNotifier extends AuthNotifier {
  @override
  Future<AuthState> build() async {
    return const AuthState(status: AuthStatus.unauthenticated);
  }
}
