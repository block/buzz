import 'dart:math' as math;

import 'package:buzz/features/profile/profile_avatar_crop_page.dart';
import 'package:flutter/foundation.dart';
import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';
import 'package:image/image.dart' as image;

void main() {
  for (final sourceSize in [const Size(160, 100), const Size(100, 160)]) {
    testWidgets('crop selection survives resizing for $sourceSize', (
      tester,
    ) async {
      tester.view.devicePixelRatio = 1;
      tester.view.physicalSize = const Size(400, 800);
      addTearDown(tester.view.resetPhysicalSize);
      addTearDown(tester.view.resetDevicePixelRatio);
      final semantics = tester.ensureSemantics();
      final bytes = image.encodePng(
        image.Image(
          width: sourceSize.width.toInt(),
          height: sourceSize.height.toInt(),
        ),
      );
      final viewerFinder = find.byKey(const ValueKey('avatar-crop-viewer'));
      await tester.runAsync(() async {
        await tester.pumpWidget(
          ProviderScope(
            child: MaterialApp(
              home: ProfileAvatarCropPage(
                imageBytes: Future<Uint8List?>.value(bytes),
              ),
            ),
          ),
        );
        // Observe image preparation completing across its compute boundary.
        final deadline = DateTime.now().add(const Duration(seconds: 10));
        while (viewerFinder.evaluate().isEmpty &&
            DateTime.now().isBefore(deadline)) {
          await Future<void>(() {});
          await tester.pump();
        }
      });
      expect(viewerFinder, findsOneWidget);
      await tester.pumpAndSettle();

      Semantics cropSemantics() => tester.widget<Semantics>(
        find.byWidgetPredicate(
          (widget) =>
              widget is Semantics && widget.properties.label == 'Photo crop',
        ),
      );
      // Exercise the editor's accessible zoom controls, then a real pan.
      for (var i = 0; i < 8; i++) {
        final actions = cropSemantics().properties.customSemanticsActions!;
        actions.entries
            .singleWhere((entry) => entry.key.label == 'Zoom in')
            .value();
        await tester.pump();
      }
      await tester.drag(viewerFinder, const Offset(48, -36));
      await tester.pumpAndSettle();

      Rect selectedSourceRect() {
        final viewer = tester.widget<InteractiveViewer>(viewerFinder);
        final canvas = tester.getSize(viewerFinder);
        final photo = viewer.child as Image;
        final matrix = viewer.transformationController!.value;
        final scale = matrix.getMaxScaleOnAxis();
        final diameter = math.min(canvas.width, canvas.height);
        // Express the visible crop in original source-image coordinates.
        final pixelsPerPoint = sourceSize.width / photo.width!;
        return Rect.fromLTWH(
          ((canvas.width - diameter) / 2 - matrix.storage[12]) /
              scale *
              pixelsPerPoint,
          ((canvas.height - diameter) / 2 - matrix.storage[13]) /
              scale *
              pixelsPerPoint,
          diameter / scale * pixelsPerPoint,
          diameter / scale * pixelsPerPoint,
        );
      }

      final selected = selectedSourceRect();
      final zoom = cropSemantics().properties.value;
      expect(selected.width, lessThan(sourceSize.shortestSide));
      expect(selected.center.dx, isNot(closeTo(sourceSize.width / 2, 0.01)));
      expect(selected.center.dy, isNot(closeTo(sourceSize.height / 2, 0.01)));
      final scale = tester
          .widget<InteractiveViewer>(viewerFinder)
          .transformationController!
          .value
          .getMaxScaleOnAxis();
      for (final size in [
        const Size(800, 600), // Expanded posture: different crop diameter.
        const Size(800, 400), // Rotation: short canvas height.
        const Size(400, 800), // Fold back without accumulating drift.
      ]) {
        tester.view.physicalSize = size;
        await tester.pumpAndSettle();
        final actual = selectedSourceRect();
        expect(actual.left, closeTo(selected.left, 0.001));
        expect(actual.top, closeTo(selected.top, 0.001));
        expect(actual.width, closeTo(selected.width, 0.001));
        expect(actual.height, closeTo(selected.height, 0.001));
        expect(actual.left, greaterThanOrEqualTo(0));
        expect(actual.top, greaterThanOrEqualTo(0));
        expect(actual.right, lessThanOrEqualTo(sourceSize.width));
        expect(actual.bottom, lessThanOrEqualTo(sourceSize.height));
        expect(
          tester
              .widget<InteractiveViewer>(viewerFinder)
              .transformationController!
              .value
              .getMaxScaleOnAxis(),
          closeTo(scale, 0.001),
        );
        expect(cropSemantics().properties.value, zoom);
        expect(tester.takeException(), isNull);
      }
      semantics.dispose();
    });
  }
}
