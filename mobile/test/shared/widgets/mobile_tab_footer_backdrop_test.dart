import 'package:buzz/shared/widgets/mobile_tab_footer_backdrop.dart';
import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';

void main() {
  testWidgets('shared gradient fades from transparent to the page surface', (
    tester,
  ) async {
    LinearGradient? gradient;

    await tester.pumpWidget(
      MaterialApp(
        home: Builder(
          builder: (context) {
            gradient = mobileTabFooterBackdropGradient(context);
            return const SizedBox();
          },
        ),
      ),
    );

    expect(gradient?.stops, [0, 0.18, 0.38, 0.6, 0.8, 1]);
    expect(gradient?.colors.first.a, 0);
    expect(gradient?.colors[1].a, closeTo(0.03, 0.01));
    expect(gradient?.colors[3].a, closeTo(0.34, 0.01));
    expect(gradient?.colors.last.a, 1);
  });

  testWidgets('uses a fixed 180px footer backdrop height', (tester) async {
    double? height;

    await tester.pumpWidget(
      Builder(
        builder: (context) {
          height = mobileTabFooterBackdropHeight(context);
          return const SizedBox();
        },
      ),
    );

    expect(height, 180);
  });
}
