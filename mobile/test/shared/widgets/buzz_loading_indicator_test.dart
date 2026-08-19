import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';
import 'package:buzz/shared/theme/theme.dart';
import 'package:buzz/shared/widgets/buzz_loading_indicator.dart';

Widget _testable({required bool disableAnimations}) {
  return ProviderScope(
    child: MaterialApp(
      theme: AppTheme.light(),
      home: MediaQuery(
        data: const MediaQueryData().copyWith(
          disableAnimations: disableAnimations,
        ),
        child: const Scaffold(
          body: BuzzLoadingIndicator(semanticLabel: 'Loading photos'),
        ),
      ),
    ),
  );
}

void main() {
  testWidgets('animates the shared arc spinner', (tester) async {
    final semantics = tester.ensureSemantics();

    await tester.pumpWidget(_testable(disableAnimations: false));
    await tester.pump(const Duration(milliseconds: 70));

    final spinner = tester.widget<RotationTransition>(
      find.byKey(const ValueKey('buzz-loading-indicator-spinner')),
    );
    expect(spinner.turns.value, greaterThan(0));
    expect(find.bySemanticsLabel('Loading photos'), findsOneWidget);
    semantics.dispose();
  });

  testWidgets('holds a static pose when reduced motion is enabled', (
    tester,
  ) async {
    await tester.pumpWidget(_testable(disableAnimations: true));
    await tester.pump(const Duration(milliseconds: 70));

    final spinner = tester.widget<RotationTransition>(
      find.byKey(const ValueKey('buzz-loading-indicator-spinner')),
    );
    expect(spinner.turns.value, 0);
    expect(tester.binding.hasScheduledFrame, isFalse);
  });
}
