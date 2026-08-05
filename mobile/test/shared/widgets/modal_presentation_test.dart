import 'package:buzz/shared/theme/theme.dart';
import 'package:buzz/shared/widgets/modal_presentation.dart';
import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';

void main() {
  testWidgets('bottom sheets use the shared 44 point close control', (
    tester,
  ) async {
    await tester.pumpWidget(
      MaterialApp(
        theme: AppTheme.light(),
        home: Scaffold(
          body: Builder(
            builder: (context) => FilledButton(
              onPressed: () => showBuzzModalBottomSheet<void>(
                context: context,
                showDragHandle: true,
                builder: (_) => const Text('Sheet body'),
              ),
              child: const Text('Open sheet'),
            ),
          ),
        ),
      ),
    );

    await tester.tap(find.text('Open sheet'));
    await tester.pumpAndSettle();

    final closeButton = find.byTooltip('Close sheet');
    expect(closeButton, findsOneWidget);
    expect(tester.getSize(closeButton), const Size.square(44));
    final closeGutter = find.ancestor(
      of: closeButton,
      matching: find.byWidgetPredicate(
        (widget) =>
            widget is Padding &&
            widget.padding ==
                const EdgeInsets.only(
                  top: Grid.gutter,
                  right: Grid.gutter,
                  bottom: Grid.xs,
                ),
      ),
    );
    expect(closeGutter, findsOneWidget);
    final gutterRect = tester.getRect(closeGutter);
    final closeRect = tester.getRect(closeButton);
    expect(closeRect.top - gutterRect.top, Grid.gutter);
    expect(gutterRect.right - closeRect.right, Grid.gutter);
    expect(
      tester.widget<BottomSheet>(find.byType(BottomSheet)).showDragHandle,
      isFalse,
    );
    expect(find.text('Sheet body'), findsOneWidget);

    await tester.tap(closeButton);
    await tester.pumpAndSettle();

    expect(find.text('Sheet body'), findsNothing);
  });
}
