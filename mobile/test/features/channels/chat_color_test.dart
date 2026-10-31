import 'package:buzz/features/channels/local_send_bubble_surface.dart';
import 'package:buzz/features/channels/message_content.dart';
import 'package:buzz/shared/theme/theme.dart';
import 'package:flutter/material.dart';
import 'package:gpt_markdown/gpt_markdown.dart';
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
      expect(bubbles[1].color, receivedMessageSurface(brightness));
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

  for (final brightness in Brightness.values) {
    for (final choice in chatColorChoices) {
      testWidgets('markdown stays readable on ${choice.name} in $brightness', (
        tester,
      ) async {
        final theme = mobileAppearanceTheme(brightness, choice.wireValue);
        final ink = theme.extension<ChatColors>()!.foreground;
        expect(
          ink,
          chatForegroundForScheme(
            theme.colorScheme,
            chatColorIndex(choice.wireValue),
          ),
        );
        final expectedLink = ink;
        await tester.pumpWidget(
          ProviderScope(
            child: MaterialApp(
              theme: theme,
              home: const AppMarkdownTheme(
                child: Scaffold(
                  body: SingleChildScrollView(
                    child: MessageContent(
                      content:
                          '# Heading\n\n[Link](https://example.com) and `code`\n\n'
                          '> Quote\n\n| Header |\n| --- |\n| Cell |\n\n'
                          '```dart\nfinal x = 1;\n```',
                      bubble: true,
                      ownMessage: true,
                    ),
                  ),
                ),
              ),
            ),
          ),
        );
        await tester.pumpAndSettle();
        final markdown = find.byType(GptMarkdown);
        final context = tester.element(markdown);
        final data = GptMarkdownTheme.of(context);
        expect(data.h1?.color, ink);
        expect(data.linkColor, expectedLink);
        expect(data.inlineCode.color, ink);
        expect(Theme.of(context).colorScheme.primary, expectedLink);
        final sheet = tester.widget<GptMarkdown>(markdown).styleSheet!;
        expect(sheet.blockQuote?.barColor, ink);
        expect(sheet.table?.headerTextStyle?.color, ink);
        // Code ink must stay readable on the composited tint, not just the fill.
        final codes = <CodeTextSpan>[];
        for (final rich in tester.widgetList<RichText>(
          find.byWidgetPredicate((widget) => widget is RichText),
        )) {
          rich.text.visitChildren((span) {
            if (span is CodeTextSpan && span.text == 'code') codes.add(span);
            return true;
          });
        }
        for (final label in ['Heading', 'Quote', 'Header', 'Cell']) {
          final renderedColors = <Color?>[];
          void inspect(InlineSpan span, TextStyle? inherited) {
            final effective = inherited?.merge(span.style) ?? span.style;
            if (span is TextSpan) {
              if (span.text?.trim() == label) {
                renderedColors.add(effective?.color);
              }
              for (final child in span.children ?? const <InlineSpan>[]) {
                inspect(child, effective);
              }
            }
          }

          for (final rich in tester.widgetList<RichText>(
            find.byWidgetPredicate((widget) => widget is RichText),
          )) {
            inspect(rich.text, null);
          }
          expect(renderedColors, isNotEmpty, reason: label);
          expect(renderedColors, everyElement(ink), reason: label);
        }
        expect(codes, hasLength(1));
        expect(codes.single.style?.color, ink);
        final fill = theme.extension<ChatColors>()!.outgoing;
        expect(
          _contrast(
            ink,
            Color.alphaBlend(codes.single.codeStyle.backgroundColor!, fill),
          ),
          greaterThanOrEqualTo(4.5),
          reason: 'inline code',
        );
        final label = find.text('dart');
        final block = tester.widget<Container>(
          find
              .ancestor(
                of: label,
                matching: find.byWidgetPredicate(
                  (widget) =>
                      widget is Container &&
                      widget.decoration is BoxDecoration &&
                      (widget.decoration! as BoxDecoration).borderRadius !=
                          null,
                ),
              )
              .first,
        );
        final blockSurface = Color.alphaBlend(
          (block.decoration! as BoxDecoration).color!,
          fill,
        );
        for (final (name, color) in [
          ('code block', ink),
          ('language label', tester.widget<Text>(label).style!.color!),
        ]) {
          expect(
            _contrast(Color.alphaBlend(color, blockSurface), blockSurface),
            greaterThanOrEqualTo(4.5),
            reason: name,
          );
        }
        expect(tester.takeException(), isNull);
      });
    }
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

double _contrast(Color a, Color b) {
  final la = a.computeLuminance();
  final lb = b.computeLuminance();
  return la > lb ? (la + .05) / (lb + .05) : (lb + .05) / (la + .05);
}
