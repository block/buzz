import 'dart:async';

import 'package:buzz/features/profile/profile_avatar_crop_page.dart';
import 'package:buzz/shared/widgets/frosted_app_bar.dart';
import 'package:buzz/shared/widgets/duo_toolbar.dart';
import 'package:buzz/shared/widgets/ios_navigation_bar.dart';
import 'package:duo_kit/duo_kit.dart';
import 'package:flutter/foundation.dart';
import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';

void main() {
  for (final side in [null, FoldStripSide.left, FoldStripSide.right]) {
    for (final failed in [false, true]) {
      testWidgets('crop has one cancel owner: $side, failed=$failed', (
        tester,
      ) async {
        debugDefaultTargetPlatformOverride = TargetPlatform.iOS;
        addTearDown(() => debugDefaultTargetPlatformOverride = null);
        final semantics = tester.ensureSemantics();
        final bytes = Completer<Uint8List?>();
        final strip = side == null
            ? null
            : FoldStrip(
                side: side,
                rect: Rect.fromLTWH(
                  side == FoldStripSide.left ? 0 : 724,
                  0,
                  76,
                  600,
                ),
                topClearance: 110,
                bottomClearance: 0,
                isClearanceEstimated: false,
              );
        await tester.pumpWidget(
          ProviderScope(
            child: MaterialApp(
              home: Builder(
                builder: (context) => TextButton(
                  onPressed: () => Navigator.of(context).push<void>(
                    MaterialPageRoute<void>(
                      builder: (_) => DuoToolbarScope(
                        strip: strip,
                        active: true,
                        child: ProfileAvatarCropPage(imageBytes: bytes.future),
                      ),
                    ),
                  ),
                  child: const Text('Open crop'),
                ),
              ),
            ),
          ),
        );
        await tester.tap(find.text('Open crop'));
        await tester.pump();
        await tester.pump(const Duration(milliseconds: 400));
        if (failed) {
          bytes.completeError(StateError('Cannot load image'));
          await tester.pump();
          await tester.pump();
          expect(find.text("We couldn't prepare that photo."), findsOneWidget);
        }
        final header = tester.widget<FrostedAppBar>(find.byType(FrostedAppBar));
        expect(header.useNativeNavigation, isTrue);
        expect(header.automaticallyImplyLeading, isFalse);
        expect(find.bySemanticsLabel(RegExp('Cancel|Back')), findsNothing);
        final bars = tester.widgetList<IosNavigationBar>(
          find.byType(IosNavigationBar),
        );
        final titleBar = bars.singleWhere((bar) => !bar.standaloneControl);
        expect(titleBar.title, 'Position Photo');
        final cancelBar = side == null
            ? titleBar
            : bars.singleWhere(
                (bar) =>
                    bar.standaloneControl &&
                    bar.leading?.label == 'Cancel photo positioning',
              );
        final done = side == null
            ? titleBar.actions.single
            : bars
                  .singleWhere(
                    (bar) =>
                        bar.standaloneControl && bar.leading?.label == 'Done',
                  )
                  .leading!;
        expect(done.onPressed, isNull);
        expect(
          cancelBar.leading!.label,
          side == null ? 'Back' : 'Cancel photo positioning',
        );
        cancelBar.leading!.onPressed!();
        await tester.pumpAndSettle();
        expect(find.byType(ProfileAvatarCropPage), findsNothing);
        expect(find.text('Open crop'), findsOneWidget);
        expect(tester.takeException(), isNull);
        semantics.dispose();
        debugDefaultTargetPlatformOverride = null;
      });
    }
  }
}
