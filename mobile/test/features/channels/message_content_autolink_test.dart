import 'package:buzz/features/channels/message_content.dart';
import 'package:buzz/shared/theme/theme.dart';
import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';
import 'package:plugin_platform_interface/plugin_platform_interface.dart';
import 'package:url_launcher_platform_interface/url_launcher_platform_interface.dart';

class _FakeUrlLauncher extends Fake
    with MockPlatformInterfaceMixin
    implements UrlLauncherPlatform {
  final launched = <String>[];

  @override
  Future<bool> launchUrl(String url, LaunchOptions options) async {
    launched.add(url);
    return true;
  }
}

final _url =
    'buzz://message?channel=580ca78b-9dae-46f3-8854-bd671853ba32'
    '&id=${'ab' * 32}';

Future<void> _pump(WidgetTester tester, String content) {
  return tester.pumpWidget(
    ProviderScope(
      child: MaterialApp(
        theme: AppTheme.light(),
        home: Scaffold(
          body: SingleChildScrollView(child: MessageContent(content: content)),
        ),
      ),
    ),
  );
}

/// Every link MessageContent drew: Buzz permalinks with a canonical label
/// render as chips keyed `buzz-link-chip:<url>`, everything else as a link
/// widget keyed `message-link:<url>`.
List<String> _links(WidgetTester tester) {
  const prefixes = ['message-link:', 'buzz-link-chip:'];
  final links = <String>[];
  for (final element
      in find
          .byWidgetPredicate((widget) => widget.key is ValueKey<String>)
          .evaluate()) {
    final key = (element.widget.key! as ValueKey<String>).value;
    for (final prefix in prefixes) {
      if (key.startsWith(prefix)) links.add(key.substring(prefix.length));
    }
  }
  return links;
}

String _plainText(WidgetTester tester) => tester
    .widgetList<RichText>(
      find.byWidgetPredicate((widget) => widget is RichText),
    )
    .map((widget) => widget.text.toPlainText())
    .join('\n');

void main() {
  testWidgets('links bare and angle-bracketed Buzz URLs', (tester) async {
    await _pump(tester, 'See $_url and <$_url>');

    expect(_links(tester), [_url, _url]);
  });

  testWidgets('keeps punctuation and Markdown delimiters out of links', (
    tester,
  ) async {
    await _pump(tester, '**open $_url**. and **_${_url}_**!');

    expect(_links(tester), [_url, _url]);
    expect(_plainText(tester), isNot(contains('*')));
    expect(_plainText(tester), isNot(contains('_!')));
  });

  group('code boundaries', () {
    final cases = <({String name, String input})>[
      (name: 'single-backtick inline span', input: '`$_url` then $_url'),
      (
        name: 'matching multi-backtick inline span',
        input: '``$_url`` then $_url',
      ),
      (
        name: 'literal shorter backtick run in inline span',
        input: '``inside ` $_url`` then $_url',
      ),
      (
        name: 'fence accepts a longer line-start closer',
        input: '```\n$_url\n````\n$_url',
      ),
      (
        name: 'fence ignores an inline-looking backtick run',
        input: '```\n$_url ``` still code\n```\n$_url',
      ),
    ];

    for (final testCase in cases) {
      testWidgets('${testCase.name} stays code', (tester) async {
        await _pump(tester, testCase.input);

        expect(_links(tester), [_url]);
        expect(_plainText(tester), contains(_url));
      });
    }
  });

  testWidgets('links bare HTTP(S) URLs without trailing punctuation', (
    tester,
  ) async {
    const httpUrl = 'https://example.com/search?q=why';
    await _pump(tester, 'See $httpUrl? and <$httpUrl?>');

    expect(_links(tester), unorderedEquals([httpUrl, '$httpUrl?']));
  });

  testWidgets('links every bare Buzz entity permalink family', (tester) async {
    final owner = 'ab' * 32;
    final id = 'cd' * 32;
    final links = [
      'buzz://repo?owner=$owner&d=buzz',
      'buzz://pr?id=$id&owner=$owner&d=buzz',
      'buzz://issue?id=$id&owner=$owner&d=buzz',
    ];
    await _pump(tester, links.map((link) => '$link.').join('\n\n'));

    expect(_links(tester), unorderedEquals(links));
  });

  testWidgets('opens email links', (tester) async {
    final launcher = _FakeUrlLauncher();
    final original = UrlLauncherPlatform.instance;
    UrlLauncherPlatform.instance = launcher;
    addTearDown(() => UrlLauncherPlatform.instance = original);
    await _pump(tester, 'Write to a@b.com');

    expect(_links(tester), ['mailto:a@b.com']);
    await tester.tapOnText(find.textRange.ofSubstring('a@b.com'));
    expect(launcher.launched, ['mailto:a@b.com']);

    // A hold offers the same Open as the tap.
    await tester.longPress(find.text('a@b.com'));
    await tester.pumpAndSettle();
    await tester.tap(find.text('Open link'));
    await tester.pumpAndSettle();
    expect(launcher.launched, ['mailto:a@b.com', 'mailto:a@b.com']);
  });

  testWidgets('links URLs with a port, a single-label host or an @', (
    tester,
  ) async {
    // gpt_markdown's bare autolink rejects these; GFM and desktop link them.
    await _pump(
      tester,
      'See http://localhost:3000/x. Or (https://example.com:8443/a), '
      'https://mastodon.social/@alice! and http://localhost:3000/a\\_b',
    );
    expect(
      _links(tester),
      unorderedEquals([
        'http://localhost:3000/x',
        'https://example.com:8443/a',
        'https://mastodon.social/@alice',
        'http://localhost:3000/a_b',
      ]),
    );
  });

  testWidgets('resolves escapes inside link URLs', (tester) async {
    // Stand-ins would become gpt_markdown placeholders inside a URL.
    await _pump(
      tester,
      '[wiki](https://en.wikipedia.org/wiki/Foo_\\(bar\\)) '
      '[x](https://e.com/a\\_b) and https://e.com/c\\_d end',
    );
    expect(
      _links(tester),
      unorderedEquals([
        'https://en.wikipedia.org/wiki/Foo_%28bar%29',
        'https://e.com/a_b',
        'https://e.com/c_d',
      ]),
    );
    expect(find.textContaining('\u2060', findRichText: true), findsNothing);
  });

  testWidgets('bare and authored links share the link colour', (tester) async {
    await _pump(tester, '[authored](https://e.com/a) https://e.com/bare');
    Color? colorOf(String label) {
      final rich = tester.widget<RichText>(
        find.text(label, findRichText: true),
      );
      Color? found;
      void walk(InlineSpan span, TextStyle? inherited) {
        final effective = inherited?.merge(span.style) ?? span.style;
        if (span is! TextSpan) return;
        if (span.text == label) found = effective?.color;
        for (final child in span.children ?? const <InlineSpan>[]) {
          walk(child, effective);
        }
      }

      walk(rich.text, null);
      return found;
    }

    expect(colorOf('https://e.com/bare'), isNotNull);
    expect(colorOf('https://e.com/bare'), colorOf('authored'));
  });

  testWidgets('a Buzz URL of an unknown shape is plain text', (tester) async {
    await _pump(tester, '[label](buzz://unknown?x=1) and buzz://unknown?y=2');
    expect(_links(tester), isEmpty);
    final paragraph = tester.widget<RichText>(
      find.textContaining('label and buzz://unknown?y=2', findRichText: true),
    );
    bool styled(InlineSpan span) =>
        span.style?.decoration == TextDecoration.underline ||
        (span is TextSpan && (span.children?.any(styled) ?? false));
    expect(styled(paragraph.text), isFalse);
  });
}
