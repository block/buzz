import 'dart:io';
import 'dart:ui' as ui;

import 'package:buzz/shared/theme/buzz_icons.dart';
import 'package:buzz/shared/theme/theme.dart';
import 'package:buzz/shared/widgets/tabler_star_icon.dart';
import 'package:flutter/material.dart';
import 'package:flutter/rendering.dart';
import 'package:flutter/services.dart';
import 'package:flutter_test/flutter_test.dart';

void main() {
  for (final dark in [false, true]) {
    testWidgets('Tabler action artwork in ${dark ? 'dark' : 'light'}', (
      tester,
    ) async {
      await tester.runAsync(() async {
        await (FontLoader(
          'Inter',
        )..addFont(rootBundle.load('assets/fonts/InterVariable.ttf'))).load();
        await (FontLoader(
          'BuzzTabler',
        )..addFont(rootBundle.load('assets/fonts/TablerIcons.ttf'))).load();
      });
      final theme = dark ? AppTheme.dark() : AppTheme.light();
      final color = theme.colorScheme.onSurface;
      final key = GlobalKey();
      await tester.pumpWidget(
        MaterialApp(
          theme: theme.copyWith(
            listTileTheme: ListTileThemeData(
              titleTextStyle: TextStyle(
                fontFamily: 'Inter',
                fontSize: 16,
                color: color,
              ),
            ),
          ),
          home: Center(
            child: RepaintBoundary(
              key: key,
              child: Material(
                child: SizedBox(
                  width: 320,
                  child: Column(
                    mainAxisSize: MainAxisSize.min,
                    children: [
                      const ListTile(
                        leading: Icon(BuzzIcons.archive),
                        title: Text('Archive'),
                      ),
                      const ListTile(
                        leading: Icon(BuzzIcons.archiveRestore),
                        title: Text('Unarchive'),
                      ),
                      const ListTile(
                        leading: Icon(BuzzIcons.clock3),
                        title: Text('Running'),
                      ),
                      const ListTile(
                        leading: Icon(BuzzIcons.clockFading),
                        title: Text('Temporary channel'),
                      ),
                      ListTile(
                        leading: TablerStarIcon(color: color),
                        title: const Text('Star'),
                      ),
                      ListTile(
                        leading: TablerStarIcon(
                          color: theme.colorScheme.primary,
                          filled: true,
                        ),
                        title: const Text('Unstar'),
                      ),
                    ],
                  ),
                ),
              ),
            ),
          ),
        ),
      );
      await tester.pumpAndSettle();
      expect(tester.takeException(), isNull);
      final directory = Platform.environment['TABLER_SCREENSHOTS'];
      if (directory != null) {
        await tester.runAsync(() async {
          final boundary =
              key.currentContext!.findRenderObject()! as RenderRepaintBoundary;
          final image = await boundary.toImage(pixelRatio: 2);
          final bytes = await image.toByteData(format: ui.ImageByteFormat.png);
          await Directory(directory).create(recursive: true);
          await File(
            '$directory/${dark ? 'dark' : 'light'}.png',
          ).writeAsBytes(bytes!.buffer.asUint8List());
          image.dispose();
        });
      }
    });
  }
}
