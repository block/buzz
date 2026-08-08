import 'package:buzz/shared/widgets/mobile_tab_footer_backdrop.dart';
import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';

void main() {
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
