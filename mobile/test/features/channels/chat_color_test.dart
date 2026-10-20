import 'package:buzz/features/channels/local_send_bubble_surface.dart';
import 'package:buzz/features/channels/message_content.dart';
import 'package:buzz/shared/theme/theme.dart';
import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';

void main() {
  for (final brightness in Brightness.values) {
    testWidgets('chat color reaches own text bubbles in $brightness', (
      tester,
    ) async {
      final theme = mobileAppearanceTheme(brightness, '#22c55e');
      await tester.pumpWidget(
        ProviderScope(
          child: MaterialApp(
            theme: theme,
            home: const Scaffold(
              body: Column(
                children: [
                  MessageContent(
                    content: 'My message',
                    bubble: true,
                    ownMessage: true,
                  ),
                  MessageContent(content: 'Their message', bubble: true),
                ],
              ),
            ),
          ),
        ),
      );
      await tester.pumpAndSettle();
      final bubbles = tester
          .widgetList<LocalSendBubbleSurface>(
            find.byType(LocalSendBubbleSurface),
          )
          .toList();
      expect(bubbles[0].color, theme.extension<ChatColors>()!.outgoing);
      expect(bubbles[1].color, theme.colorScheme.secondaryContainer);
      final text = tester
          .widgetList<RichText>(find.byType(RichText))
          .firstWhere((text) => text.text.toPlainText() == 'My message');
      final colors = <Color?>[];
      void inspect(InlineSpan span, TextStyle? inherited) {
        final effective = inherited?.merge(span.style) ?? span.style;
        if (span is TextSpan) {
          if (span.text?.contains('My message') == true) {
            colors.add(effective?.color);
          }
          for (final child in span.children ?? const <InlineSpan>[]) {
            inspect(child, effective);
          }
        }
      }

      inspect(text.text, null);
      expect(colors, [Colors.white]);
    });
  }

  testWidgets('System follows platform brightness with the same chat choice', (
    tester,
  ) async {
    addTearDown(tester.platformDispatcher.clearPlatformBrightnessTestValue);
    Color? surface;
    Color? bubble;
    await tester.pumpWidget(
      MaterialApp(
        theme: mobileAppearanceTheme(Brightness.light, '#ef4444'),
        darkTheme: mobileAppearanceTheme(Brightness.dark, '#ef4444'),
        themeMode: ThemeMode.system,
        home: Builder(
          builder: (context) {
            surface = Theme.of(context).colorScheme.surface;
            bubble = ChatColors.of(context).outgoing;
            return const SizedBox();
          },
        ),
      ),
    );
    for (final brightness in [Brightness.dark, Brightness.light]) {
      tester.platformDispatcher.platformBrightnessTestValue = brightness;
      await tester.pumpAndSettle();
      final expected = mobileAppearanceTheme(brightness, '#ef4444');
      expect(surface, expected.colorScheme.surface);
      expect(bubble, expected.extension<ChatColors>()!.outgoing);
    }
  });
}
