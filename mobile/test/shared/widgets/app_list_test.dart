import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:buzz/shared/theme/theme.dart';
import 'package:buzz/shared/widgets/app_list.dart';
import 'package:buzz/shared/widgets/app_list_card.dart';

Widget _host(Widget child) => MaterialApp(
  theme: AppTheme.light(),
  home: Scaffold(body: child),
);

void main() {
  group('AppListRow', () {
    testWidgets('centres the leading icon against a two-line row', (
      tester,
    ) async {
      await tester.pumpWidget(
        _host(
          const AppListRow(
            icon: Icons.dns,
            title: 'Connected to',
            subtitle: 'https://relay.example.com',
          ),
        ),
      );

      final row = tester.getRect(find.byType(AppListRow));
      final icon = tester.getRect(find.byType(Icon));
      expect(icon.center.dy, closeTo(row.center.dy, 0.5));
    });

    testWidgets('centres the trailing control against a two-line row', (
      tester,
    ) async {
      await tester.pumpWidget(
        _host(
          const AppListRow(
            icon: Icons.key,
            title: 'Identity',
            subtitle: 'deadbeef',
            trailing: Icon(Icons.copy, key: Key('trailing')),
          ),
        ),
      );

      final row = tester.getRect(find.byType(AppListRow));
      final trailing = tester.getRect(find.byKey(const Key('trailing')));
      expect(trailing.center.dy, closeTo(row.center.dy, 0.5));
    });

    testWidgets('keeps the value flush against the trailing edge', (
      tester,
    ) async {
      await tester.pumpWidget(
        _host(
          const AppListRow(
            icon: Icons.palette,
            title: 'Theme',
            value: 'Buzz',
            trailing: Icon(Icons.chevron_right, key: Key('chevron')),
          ),
        ),
      );

      final value = tester.getRect(find.text('Buzz'));
      final chevron = tester.getRect(find.byKey(const Key('chevron')));
      // Only the row's own inter-widget gap sits between them — no flex slack.
      expect(chevron.left - value.right, closeTo(Grid.xxs, 0.5));
    });
  });

  group('AppListCard', () {
    testWidgets('separates rows with dividers that clear the icon column', (
      tester,
    ) async {
      await tester.pumpWidget(
        _host(
          const AppListCard(
            label: 'Style',
            children: [
              AppListRow(icon: Icons.sunny, title: 'Appearance'),
              AppListRow(icon: Icons.palette, title: 'Theme'),
              AppListRow(icon: Icons.water_drop, title: 'Accent color'),
            ],
          ),
        ),
      );

      final dividers = find.byType(Divider);
      expect(dividers, findsNWidgets(2));

      final divider = tester.widget<Divider>(dividers.first);
      final scheme = AppTheme.light().colorScheme;
      // Derived from the text color, not the scheme's border tokens — those sit
      // within a few levels of the card fill and vanish.
      expect(divider.color, scheme.onSurface.withValues(alpha: 0.12));

      // The indent is painted inside the divider's box, so the line starts at
      // the box's left edge plus the indent — level with the row titles.
      final titleLeft = tester.getRect(find.text('Appearance')).left;
      final lineLeft = tester.getRect(dividers.first).left + divider.indent!;
      expect(lineLeft, closeTo(titleLeft, 0.5));
    });

    testWidgets('fills with the softest container step', (tester) async {
      await tester.pumpWidget(
        _host(
          const AppListCard(children: [AppListRow(title: 'Remove community')]),
        ),
      );

      final material = tester.widget<Material>(
        find.descendant(
          of: find.byType(AppListCard),
          matching: find.byType(Material),
        ),
      );
      expect(
        material.color,
        AppTheme.light().colorScheme.surfaceContainerHighest,
      );
    });
  });
}
