import 'package:buzz/shared/theme/theme.dart';
import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';

double _contrast(Color a, Color b) {
  final la = a.computeLuminance();
  final lb = b.computeLuminance();
  return la > lb ? (la + .05) / (lb + .05) : (lb + .05) / (la + .05);
}

void main() {
  /// Builds a context carrying the app theme, the same way the app does.
  Future<BuildContext> pumpTheme(WidgetTester tester, bool dark) async {
    late BuildContext captured;
    await tester.pumpWidget(
      MaterialApp(
        theme: dark ? AppTheme.dark() : AppTheme.light(),
        home: Builder(
          builder: (context) {
            captured = context;
            return const SizedBox();
          },
        ),
      ),
    );
    return captured;
  }

  for (final dark in [false, true]) {
    testWidgets(
      'availability colors stay visible on the ${dark ? 'dark' : 'light'} '
      'surface',
      (tester) async {
        final context = await pumpTheme(tester, dark);
        final surface = context.colors.surface;
        for (final presence in const ['online', 'away', 'offline', null]) {
          expect(
            _contrast(context.presenceColor(presence), surface),
            greaterThanOrEqualTo(3),
            reason: 'presence ${presence ?? 'unknown'} is too faint for a dot',
          );
        }
        // The online and away dots stay the semantic status colors.
        expect(context.presenceColor('online'), context.appColors.success);
        expect(context.presenceColor('away'), context.appColors.warning);
        // Offline is a foreground token, never the hairline-border `outline`.
        expect(
          context.presenceColor('offline'),
          context.colors.onSurfaceVariant,
        );
        expect(context.presenceColor('offline'), isNot(context.colors.outline));
        expect(context.presenceColor(null), context.presenceColor('offline'));
      },
    );
  }
}
