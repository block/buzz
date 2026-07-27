import 'package:buzz/shared/theme/theme.dart';
import 'package:buzz/shared/widgets/frosted_app_bar.dart';
import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';

void main() {
  testWidgets('title row and reported height grow with accessible text', (
    tester,
  ) async {
    const titleStyle = TextStyle(fontSize: 22, height: 1.3);
    late double reportedHeight;

    await tester.pumpWidget(
      MaterialApp(
        theme: AppTheme.light(),
        home: MediaQuery(
          data: const MediaQueryData(textScaler: TextScaler.linear(2)),
          child: Builder(
            builder: (context) {
              reportedHeight = frostedAppBarHeight(
                context,
                titleStyle: titleStyle,
              );
              return const Stack(
                children: [
                  FrostedAppBar(title: Text('Search'), titleStyle: titleStyle),
                ],
              );
            },
          ),
        ),
      ),
    );
    await tester.pumpAndSettle();

    final clip = find.descendant(
      of: find.byType(FrostedAppBar),
      matching: find.byType(ClipRect),
    );
    expect(reportedHeight, greaterThan(48));
    expect(tester.getSize(clip).height, closeTo(reportedHeight, 0.01));
    expect(tester.getSize(find.text('Search')).height, greaterThan(48));
    expect(tester.takeException(), isNull);
  });

  testWidgets('custom multi-line title height matches reported height', (
    tester,
  ) async {
    const titleContentHeight = 64.0;
    late double reportedHeight;

    await tester.pumpWidget(
      MaterialApp(
        theme: AppTheme.light(),
        home: Builder(
          builder: (context) {
            reportedHeight = frostedAppBarHeight(
              context,
              titleContentHeight: titleContentHeight,
            );
            return const Stack(
              children: [
                FrostedAppBar(
                  titleContentHeight: titleContentHeight,
                  title: Column(
                    mainAxisSize: MainAxisSize.min,
                    children: [Text('Title'), Text('Subtitle')],
                  ),
                ),
              ],
            );
          },
        ),
      ),
    );
    await tester.pumpAndSettle();

    final clip = find.descendant(
      of: find.byType(FrostedAppBar),
      matching: find.byType(ClipRect),
    );
    expect(tester.getSize(clip).height, closeTo(reportedHeight, 0.01));
    expect(reportedHeight, closeTo(titleContentHeight + Grid.xs + 1, 0.01));
    expect(tester.takeException(), isNull);
  });
}
