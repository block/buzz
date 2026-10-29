import 'dart:async';

import 'package:buzz/features/profile/profile_avatar_crop_page.dart';
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
        if (side == null) {
          expect(find.bySemanticsLabel('Back'), findsOneWidget);
          expect(find.byType(IosNavigationBar), findsNothing);
          await tester.tap(find.text('Back'));
        } else {
          // UIKit owns the rail's semantics; its native action payload must
          // have one cancel, with no competing Flutter cancel/back control.
          expect(find.bySemanticsLabel(RegExp('Cancel|Back')), findsNothing);
          final header = tester.widget<AppBar>(find.byType(AppBar));
          expect(header.leading, isNull);
          expect(header.automaticallyImplyLeading, isFalse);
          final native = tester.widget<UiKitView>(find.byType(UiKitView));
          final payload = native.creationParams! as Map<String, Object?>;
          expect(
            payload['leading'],
            containsPair('label', 'Cancel photo positioning'),
          );
          final rail = tester.widget<IosNavigationBar>(
            find.byType(IosNavigationBar),
          );
          rail.leading!.onPressed!();
        }
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
