import 'dart:async';
import 'package:flutter/material.dart';
import 'package:buzz/shared/community/paired_community_landing.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';
import 'package:shared_preferences/shared_preferences.dart';
import 'package:buzz/app.dart';
import 'package:buzz/features/age_gate/age_signal_provider.dart';
import 'package:buzz/shared/auth/auth.dart';
import 'package:buzz/shared/theme/theme_provider.dart';

void main() {
  testWidgets('pairing loading cover blocks hidden input and semantics', (
    tester,
  ) async {
    SharedPreferences.setMockInitialValues({});
    final prefs = await SharedPreferences.getInstance();
    final container = ProviderContainer(
      overrides: [
        authProvider.overrideWith(() => _FakeAuthNotifier()),
        ageSignalProvider.overrideWith(() => _AllowedAgeSignalNotifier()),
        savedPrefsProvider.overrideWithValue(prefs),
      ],
    );
    addTearDown(container.dispose);
    final semantics = tester.ensureSemantics();
    String semanticsTree() => tester
        .binding
        .renderViews
        .single
        .owner!
        .semanticsOwner!
        .rootSemanticsNode!
        .toStringDeep();
    await tester.pumpWidget(
      UncontrolledProviderScope(container: container, child: const App()),
    );
    await tester.pump();
    var taps = 0;
    final navigator = tester.state<NavigatorState>(
      find.byType(Navigator).first,
    );
    unawaited(
      navigator.push(
        MaterialPageRoute<void>(
          builder: (_) => Scaffold(
            body: Align(
              alignment: Alignment.topLeft,
              child: TextButton(
                onPressed: () => taps++,
                child: const Text('Hidden action'),
              ),
            ),
          ),
        ),
      ),
    );
    await tester.pumpAndSettle();
    final button = find.text('Hidden action');
    final location = tester.getCenter(button);
    expect(semanticsTree(), contains('Hidden action'));
    container
        .read(pairedCommunityLandingProvider.notifier)
        .request(
          Community(
            id: 'test',
            addedAt: DateTime(2026),
            name: 'Test',
            relayUrl: 'https://relay.test',
          ),
        );
    await tester.pump();
    await tester.tapAt(location);
    expect(taps, 0);
    expect(semanticsTree(), isNot(contains('Hidden action')));
    container.read(pairedCommunityLandingProvider.notifier).clear();
    await tester.pump();
    expect(semanticsTree(), contains('Hidden action'));
    await tester.tapAt(location);
    expect(taps, 1);
    await tester.pumpWidget(const SizedBox.shrink());
    semantics.dispose();
  });

  testWidgets('App renders pairing page when unauthenticated', (
    WidgetTester tester,
  ) async {
    SharedPreferences.setMockInitialValues({});
    final prefs = await SharedPreferences.getInstance();

    await tester.pumpWidget(
      ProviderScope(
        overrides: [
          authProvider.overrideWith(() => _FakeAuthNotifier()),
          ageSignalProvider.overrideWith(() => _AllowedAgeSignalNotifier()),
          savedPrefsProvider.overrideWithValue(prefs),
        ],
        child: const App(),
      ),
    );
    await tester.pump();
    expect(find.bySemanticsLabel('Buzz'), findsOneWidget);
    expect(find.text('Scan a QR code'), findsOneWidget);
  });
}

class _AllowedAgeSignalNotifier extends AgeSignalNotifier {
  @override
  AgeSignalState build() => AgeSignalState.allowed;

  @override
  Future<void> request() async {}
}

class _FakeAuthNotifier extends AuthNotifier {
  @override
  Future<AuthState> build() async {
    return const AuthState(status: AuthStatus.unauthenticated);
  }
}
