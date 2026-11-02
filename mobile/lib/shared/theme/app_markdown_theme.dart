import 'package:flutter/material.dart';
import 'package:gpt_markdown/gpt_markdown.dart';

import 'app_theme.dart';
import 'code_style.dart';

/// States the app's code style on every `GptMarkdown` below it and leaves the
/// rest of the Markdown theme as it already was.
///
/// Registering a [GptMarkdownThemeData] as a `ThemeExtension` would be the
/// shorter route, but its factory is not a partial override: it builds a fresh
/// stock Material theme and fills every heading, rule and highlight field from
/// it, so registering one stops `GptMarkdownTheme.of` deriving those from the
/// app. Restating them in the extension does not close the gap either — the
/// ambient headings carry the localized text geometry, which exists only once
/// `Theme.of` has run and a `ThemeData` factory cannot see. Overriding the one
/// field on the ambient theme is what leaves non-code Markdown untouched.
class AppMarkdownTheme extends StatefulWidget {
  const AppMarkdownTheme({super.key, required this.child});

  final Widget child;

  @override
  State<AppMarkdownTheme> createState() => _AppMarkdownThemeState();
}

class _AppMarkdownThemeState extends State<AppMarkdownTheme> {
  ThemeData? _theme;
  GptMarkdownThemeData? _outer;
  GptMarkdownThemeData? _data;

  @override
  Widget build(BuildContext context) {
    final theme = Theme.of(context);
    final outer = context
        .dependOnInheritedWidgetOfExactType<GptMarkdownTheme>()
        ?.gptThemeData;
    // GptMarkdownTheme notifies by data identity, and every notification makes
    // each GptMarkdown in the app re-parse its text. Derive new data only when
    // what it is derived from changes.
    if (_data == null || theme != _theme || !identical(outer, _outer)) {
      _theme = theme;
      _outer = outer;
      final scheme = theme.colorScheme;
      _data = GptMarkdownTheme.of(context).copyWith(
        // Inline code is otherwise drawn in gpt_markdown's own bundled face, at
        // its own size, tinted from `onSurface`. Stating the app's code style
        // gives every Markdown surface the face, size and chip colours a fenced
        // code block already uses.
        inlineCode: InlineCodeStyle(
          fontFamily: CodeStyle.fontFamily,
          fontSizeFactor: CodeStyle.fontSizeFactor,
          color: scheme.onSurface,
          backgroundColor: CodeStyle.background(scheme),
          borderColor: CodeStyle.border(scheme),
          // A chip is small, so it takes the smallest step of the app's radius
          // scale rather than the `Radii.card` a block uses.
          borderRadius: const Radius.circular(Radii.xs),
        ).resolve(scheme),
      );
    }
    return GptMarkdownTheme(gptThemeData: _data!, child: widget.child);
  }
}
