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

    expect(gradient?.stops, [0, 0.5, 1]);
    expect(gradient?.colors.first.a, 0);
    expect(gradient?.colors[1].a, 0.75);
    expect(gradient?.colors.last.a, 1);
  });

  testWidgets('uses the logical bottom safe-area inset', (tester) async {
    double? height;

    await tester.pumpWidget(
      MediaQuery(
        data: const MediaQueryData(padding: EdgeInsets.only(bottom: 34)),
        child: Builder(
          builder: (context) {
            height = mobileTabFooterBackdropHeight(context);
            return const SizedBox();
          },
        ),
      ),
    );

    expect(height, 170);
  });
}
