import 'package:buzz/shared/theme/buzz_icons.dart';
import 'package:buzz/shared/widgets/section_state_chevron.dart';
import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';

void main() {
  testWidgets('shows the latest section state until the last toggle expires', (
    tester,
  ) async {
    Future<void> show(bool expanded) => tester.pumpWidget(
      MaterialApp(home: SectionStateChevron(expanded: expanded)),
    );
    double opacity() =>
        tester.widget<AnimatedOpacity>(find.byType(AnimatedOpacity)).opacity;
    await show(true);
    expect(opacity(), 0);
    await show(false);
    expect(opacity(), 1);
    expect(find.byIcon(BuzzIcons.chevronRight), findsOneWidget);
    await tester.pump(const Duration(seconds: 2));
    expect(opacity(), 1);
    await show(true);
    expect(find.byIcon(BuzzIcons.chevronDown), findsOneWidget);
    await tester.pump(const Duration(milliseconds: 500));
    expect(opacity(), 1);
    await tester.pump(const Duration(milliseconds: 400));
    expect(opacity(), 0);
  });
}
