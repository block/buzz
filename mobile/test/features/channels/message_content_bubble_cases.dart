part of 'message_content_test.dart';

void bubbleContentTests() {
  final bubbles = find.byWidgetPredicate(
    (widget) =>
        widget.key is ValueKey<String> &&
        (widget.key! as ValueKey<String>).value.startsWith(
          'message-text-bubble-',
        ),
  );
  testWidgets('message shapes remain inside their bounds without tails', (
    tester,
  ) async {
    for (final tail in [false, true]) {
      await tester.pumpWidget(
        _testable(
          MessageContent(content: 'Hello', bubble: true, bubbleEnd: tail),
        ),
      );
      await tester.pumpAndSettle();
      final decoration = tester.widget<LocalSendBubbleSurface>(
        find.descendant(
          of: bubbles,
          matching: find.byType(LocalSendBubbleSurface),
        ),
      );
      final bounds = decoration.shape
          .getOuterPath(Offset.zero & tester.getSize(bubbles))
          .getBounds();
      expect(bounds.left, 0);
      expect(bounds.bottom, tester.getSize(bubbles).height);
    }
  });

  for (final dark in [false, true]) {
    testWidgets('outgoing headings, mentions and links contrast in dark=$dark', (
      tester,
    ) async {
      String? tapped;
      await tester.pumpWidget(
        _testable(
          MessageContent(
            content:
                '# H1\n\n## H2\n\n### H3\n\n#### H4\n\n##### H5\n\n###### H6\n\n@Alice @Helper [Website](https://example.com)',
            bubble: true,
            ownMessage: true,
            mentionNames: const {'person': 'Alice', 'agent': 'Helper'},
            agentMentionPubkeys: const {'agent'},
            onMentionTap: (pubkey) => tapped = pubkey,
          ),
          dark: dark,
        ),
      );
      await tester.pumpAndSettle();
      final foreground = dark ? Colors.black : Colors.white;
      for (final heading in ['H1', 'H2', 'H3', 'H4', 'H5', 'H6']) {
        final text = tester.widget<RichText>(_findRich(heading).last);
        final colors = <Color?>[];
        void inspect(InlineSpan span, TextStyle? inherited) {
          final effective = inherited?.merge(span.style) ?? span.style;
          if (span is TextSpan) {
            if (span.text?.contains(heading) == true) {
              colors.add(effective?.color);
            }
            for (final child in span.children ?? const <InlineSpan>[]) {
              inspect(child, effective);
            }
          }
        }

        inspect(text.text, null);
        expect(colors, [foreground]);
      }
      for (final label in ['Alice', 'Helper']) {
        expect(tester.widget<Text>(find.text(label)).style?.color, foreground);
      }
      expect(tester.widget<Icon>(find.byIcon(BuzzIcons.bot)).color, foreground);
      final link = tester.widget<Text>(
        find.byWidgetPredicate(
          (w) => w is Text && w.textSpan?.toPlainText() == 'Website',
        ),
      );
      expect(link.textSpan?.style?.color, foreground);
      await tester.tap(find.text('Alice'));
      expect(tapped, 'person');
      await tester.tap(find.text('Helper'));
      expect(tapped, 'agent');
    });
  }

  for (final content in [
    '- screenshots:\n  ![result](https://example.com/result.png)\n  explanation',
    '1. screenshots:\n  ![result](https://example.com/result.png)\n  explanation',
    '> screenshots:\n> ![result](https://example.com/result.png)\n> explanation',
    '> screenshots:\n![result](https://example.com/result.png)\nexplanation',
  ]) {
    testWidgets('nested media retains its Markdown block: $content', (
      tester,
    ) async {
      await tester.pumpWidget(
        _testable(MessageContent(content: content, bubble: true)),
      );
      await tester.pumpAndSettle();
      expect(bubbles, findsOneWidget);
      final markdown = tester.widget<GptMarkdown>(find.byType(GptMarkdown));
      expect(markdown.data, content);
      final media = find.byKey(
        const ValueKey(
          'message-media-image-preview:https://example.com/result.png',
        ),
      );
      expect(find.descendant(of: bubbles, matching: media), findsOneWidget);
      expect(tester.takeException(), isNull);
    });
  }

  testWidgets('image corners follow the conversation group', (tester) async {
    for (final grouped in [false, true]) {
      await tester.pumpWidget(
        _testable(
          MessageContent(
            content: '![photo](https://example.com/photo.png)',
            bubble: true,
            bubbleStart: !grouped,
            bubbleEnd: !grouped,
          ),
        ),
      );
      await tester.pumpAndSettle();
      final frame = tester.widget<Container>(
        find.byKey(
          const ValueKey(
            'message-media-image-preview:https://example.com/photo.png',
          ),
        ),
      );
      final corners =
          (frame.decoration! as BoxDecoration).borderRadius! as BorderRadius;
      expect(corners.topLeft.x, grouped ? Radii.sm : Radii.container);
      expect(corners.bottomLeft.x, grouped ? Radii.sm : Radii.container);
      expect(corners.topRight.x, Radii.container);
      expect(corners.bottomRight.x, Radii.container);
    }
  });

  testWidgets('photo gallery stays outside its caption bubble', (tester) async {
    await tester.pumpWidget(
      _testable(
        const MessageContent(
          content:
              'Photos\n![one](https://example.com/one.png)\n![two](https://example.com/two.png)',
          bubble: true,
        ),
      ),
    );
    await tester.pumpAndSettle();
    final gallery = find.byKey(const ValueKey('message-media-carousel'));
    expect(gallery, findsOneWidget);
    expect(bubbles, findsOneWidget);
    expect(find.descendant(of: bubbles, matching: gallery), findsNothing);
    expect(
      tester.getTopLeft(gallery).dy,
      greaterThanOrEqualTo(tester.getBottomLeft(bubbles).dy),
    );
  });

  testWidgets('neutral bubbles hug short text and distinguish own messages', (
    tester,
  ) async {
    for (final own in [false, true]) {
      await tester.pumpWidget(
        _testable(
          MessageContent(content: 'Hello', bubble: true, ownMessage: own),
        ),
      );
      await tester.pumpAndSettle();
      expect(bubbles, findsOneWidget);
      expect(tester.getSize(bubbles).width, lessThan(160));
      final decoration = tester.widget<LocalSendBubbleSurface>(
        find.descendant(
          of: bubbles,
          matching: find.byType(LocalSendBubbleSurface),
        ),
      );
      expect(decoration.color, isNotNull);
      expect(
        find.descendant(of: bubbles, matching: _findRich('Hello')),
        findsWidgets,
      );
    }
  });

  testWidgets('attachments stay outside bubbles and text order is preserved', (
    tester,
  ) async {
    await tester.pumpWidget(
      _testable(
        const MessageContent(
          content:
              'Before\n\n[file.txt](https://example.com/file.txt)\n\nAfter',
          tags: [
            ['imeta', 'url https://example.com/file.txt', 'm text/plain'],
          ],
          bubble: true,
        ),
      ),
    );
    await tester.pumpAndSettle();
    expect(bubbles, findsNWidgets(2));
    expect(
      find.descendant(of: bubbles, matching: _findRich('file.txt')),
      findsNothing,
    );
    expect(
      tester.getTopLeft(_findRich('Before')).dy,
      lessThan(tester.getTopLeft(_findRich('file.txt')).dy),
    );
    expect(
      tester.getTopLeft(_findRich('file.txt')).dy,
      lessThan(tester.getTopLeft(_findRich('After')).dy),
    );
  });

  testWidgets('fenced attachment examples remain in the text bubble', (
    tester,
  ) async {
    await tester.pumpWidget(
      _testable(
        const MessageContent(
          content: '```text\n![example](https://example.com/image.png)\n```',
          bubble: true,
        ),
      ),
    );
    await tester.pumpAndSettle();
    expect(bubbles, findsOneWidget);
    expect(
      find.descendant(
        of: bubbles,
        matching: _findRich('![example](https://example.com/image.png)'),
      ),
      findsWidgets,
    );
    expect(tester.takeException(), isNull);
  });

  testWidgets('bubble text reflows at narrow width and large text', (
    tester,
  ) async {
    _setSurfaceSize(tester, const Size(320, 800));
    addTearDown(tester.view.resetPhysicalSize);
    addTearDown(tester.view.resetDevicePixelRatio);
    await tester.pumpWidget(
      _testable(
        MediaQuery(
          data: const MediaQueryData(textScaler: TextScaler.linear(2)),
          child: const SizedBox(
            width: 230,
            child: MessageContent(
              content:
                  'A longer message that wraps across multiple lines without clipping.',
              bubble: true,
            ),
          ),
        ),
      ),
    );
    await tester.pumpAndSettle();
    expect(tester.getSize(bubbles).width, lessThanOrEqualTo(230));
    expect(tester.getSize(bubbles).height, greaterThan(80));
    expect(tester.takeException(), isNull);
  });
}
