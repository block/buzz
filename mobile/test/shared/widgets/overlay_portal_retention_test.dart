import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:leak_tracker/leak_tracker.dart';

void main() {
  testWidgets('disposed composer-style overlay portals are collectible', (
    tester,
  ) async {
    final semantics = tester.ensureSemantics();
    try {
      // Keep the application's semantics owner alive while each route leaves.
      // Disposing the entire app would hide retention in its traversal maps.
      final retired = <WeakReference<State<StatefulWidget>>>[];
      for (var visit = 0; visit < 3; visit++) {
        retired.add(await _visitOverlay(tester));
      }

      await tester.runAsync(() => forceGC(fullGcCycles: 3));
      expect(
        retired.where((reference) => reference.target != null),
        isEmpty,
        reason:
            'A closed route must not survive through semantics traversal keys',
      );
    } finally {
      semantics.dispose();
    }
  });
}

Future<WeakReference<State<StatefulWidget>>> _visitOverlay(
  WidgetTester tester,
) async {
  final controller = OverlayPortalController()..show();
  await tester.pumpWidget(
    MaterialApp(
      home: OverlayPortal.overlayChildLayoutBuilder(
        controller: controller,
        overlayChildBuilder: (_, _) => const Positioned(
          left: 0,
          top: 0,
          child: Text('Composer suggestions'),
        ),
        child: const Text('Composer'),
      ),
    ),
  );
  await tester.pumpAndSettle();
  final retired = WeakReference(tester.state(find.byType(OverlayPortal)));
  await tester.pumpWidget(const MaterialApp(home: SizedBox()));
  await tester.pumpAndSettle();
  return retired;
}
