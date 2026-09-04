import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:gpt_markdown/gpt_markdown.dart';
import 'package:buzz/shared/theme/app_markdown_theme.dart';
import 'package:buzz/shared/theme/app_theme.dart';
import 'package:buzz/shared/theme/code_style.dart';

/// The Markdown theme a widget reads under [theme], with [wrapped] deciding
/// whether the app states its code style above it.
Future<GptMarkdownThemeData> _markdownTheme(
  WidgetTester tester,
  ThemeData theme, {
  required bool wrapped,
}) async {
  late GptMarkdownThemeData read;
  final reader = Builder(
    builder: (context) {
      read = GptMarkdownTheme.of(context);
      return const SizedBox.shrink();
    },
  );

  await tester.pumpWidget(
    MaterialApp(
      theme: theme,
      home: wrapped ? AppMarkdownTheme(child: reader) : reader,
    ),
  );
  // MaterialApp animates a theme change, so the first frame after a pump can
  // still carry the previous one.
  await tester.pumpAndSettle();
  return read;
}

void main() {
  testWidgets('states inline code and leaves the rest of Markdown alone', (
    tester,
  ) async {
    for (final theme in [AppTheme.light(), AppTheme.dark()]) {
      final ambient = await _markdownTheme(tester, theme, wrapped: false);
      final stated = await _markdownTheme(tester, theme, wrapped: true);

      // Headings, rules and the rest keep the values gpt_markdown derives from
      // the app theme: a code-only change must not restyle anything else.
      expect(stated.h1, ambient.h1);
      expect(stated.h2, ambient.h2);
      expect(stated.h3, ambient.h3);
      expect(stated.h4, ambient.h4);
      expect(stated.h5, ambient.h5);
      expect(stated.h6, ambient.h6);
      expect(stated.hrLineColor, ambient.hrLineColor);
      expect(stated.hrLineThickness, ambient.hrLineThickness);
      expect(stated.hrLinePadding, ambient.hrLinePadding);
      expect(stated.highlightColor, ambient.highlightColor);
      expect(stated.linkColor, ambient.linkColor);
      expect(stated.linkHoverColor, ambient.linkHoverColor);
      expect(
        stated.autoAddDividerLineAfterH1,
        ambient.autoAddDividerLineAfterH1,
      );
      expect(stated.styleSheet, ambient.styleSheet);

      // Inline code is the whole of the difference.
      final scheme = theme.colorScheme;
      expect(stated.inlineCode.fontFamily, CodeStyle.fontFamily);
      expect(stated.inlineCode.color, scheme.onSurface);
      expect(stated.inlineCode.backgroundColor, CodeStyle.background(scheme));
      expect(stated.inlineCode.borderColor, CodeStyle.border(scheme));
      expect(
        ambient.inlineCode.fontFamily,
        isNot(CodeStyle.fontFamily),
        reason: 'the package default is what this widget exists to replace',
      );
    }
  });
}
