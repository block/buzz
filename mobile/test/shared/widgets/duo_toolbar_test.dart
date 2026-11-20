import 'package:buzz/shared/widgets/duo_toolbar.dart';
import 'package:buzz/shared/widgets/ios_navigation_bar.dart';
import 'package:duo_kit/duo_kit.dart';
import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';

void main() {
  testWidgets('detail rail controls clear the shared workspace capsule', (
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
    final detail = find.byKey(const ValueKey('duo-toolbar-Close'));

    Future<double> detailTop(int leadingSlots) async {
      await tester.pumpWidget(
        ProviderScope(
          child: MaterialApp(
            home: DuoToolbarScope(
              active: true,
              strip: strip,
              leadingSlots: leadingSlots,
              child: DuoToolbarPortal(
                strip: strip,
                active: true,
                actions: [
                  IosNavigationAction(label: 'Close', onPressed: () {}),
                ],
                child: const SizedBox.expand(),
              ),
            ),
          ),
        ),
      );
      await tester.pumpAndSettle();
      return tester.getTopLeft(detail).dy;
    }

    final base = await detailTop(0);
    // A lone workspace control takes one 60-point rail slot.
    expect(await detailTop(1), base + 60);
    // Two workspace controls share one capsule: 48 points each plus one
    // 12-point margin, so the gap below it matches the gap between slots.
    expect(await detailTop(2), base + 108);
  }, variant: TargetPlatformVariant.only(TargetPlatform.iOS));

  testWidgets('adjacent shared actions form one capsule sized to its group', (
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
    await tester.pumpWidget(
      ProviderScope(
        child: MaterialApp(
          home: DuoToolbarPortal(
            strip: strip,
            active: true,
            actions: [
              IosNavigationAction(
                label: 'A',
                sharesRailBackground: true,
                onPressed: () {},
              ),
              IosNavigationAction(
                label: 'B',
                sharesRailBackground: true,
                onPressed: () {},
              ),
              IosNavigationAction(label: 'C', onPressed: () {}),
            ],
            child: const SizedBox.expand(),
          ),
        ),
      ),
    );
    await tester.pumpAndSettle();
    final group = find.byKey(const ValueKey('duo-toolbar-A+B'));
    final single = find.byKey(const ValueKey('duo-toolbar-C'));
    expect(tester.getSize(group).height, 108);
    expect(tester.getSize(single).height, 60);
    expect(tester.getTopLeft(single).dy, tester.getBottomLeft(group).dy);
  }, variant: TargetPlatformVariant.only(TargetPlatform.iOS));
}
