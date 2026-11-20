import 'package:buzz/shared/widgets/duo_toolbar.dart';
import 'package:buzz/shared/widgets/ios_navigation_bar.dart';
import 'package:duo_kit/duo_kit.dart';
import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';

void main() {
  testWidgets('scrolling rail controls leave room for the bottom capsule', (
    tester,
  ) async {
    tester.view.devicePixelRatio = 1;
    tester.view.physicalSize = const Size(950, 670);
    addTearDown(tester.view.reset);
    const strip = FoldStrip(
      side: FoldStripSide.right,
      rect: Rect.fromLTWH(870, 0, 80, 670),
      topClearance: 110,
      bottomClearance: 0,
      isClearanceEstimated: false,
    );
    // Too many controls scroll, leaving room below them for the capsule that
    // Home pins to the bottom of the rail: Search, New and Bestie.
    await tester.pumpWidget(
      ProviderScope(
        child: MaterialApp(
          home: DuoToolbarPortal(
            strip: strip,
            active: true,
            actions: [
              for (var i = 0; i < 10; i++)
                IosNavigationAction(label: 'Row $i', onPressed: () {}),
            ],
            child: const SizedBox.expand(),
          ),
        ),
      ),
    );
    await tester.pumpAndSettle();
    expect(
      tester.getSize(find.byType(SingleChildScrollView)).height,
      670 - 110 - duoRailCapsuleExtent(duoRailBottomControls),
    );
    expect(duoRailCapsuleExtent(duoRailBottomControls), 156);
  }, variant: TargetPlatformVariant.only(TargetPlatform.iOS));
}
