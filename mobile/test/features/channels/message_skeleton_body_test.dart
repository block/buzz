import 'package:buzz/features/channels/message_skeleton_body.dart';
import 'package:buzz/shared/theme/theme.dart';
import 'package:buzz/shared/widgets/skeleton.dart';
import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';

void main() {
  for (final (mime, name, kind) in [
    ('image/jpeg', 'photo.jpg', 'image'),
    ('video/mp4', 'clip.mp4', 'video'),
    ('audio/mp4', 'voice.m4a', 'audio'),
    ('video/mp4', 'voice-note-123.mp4', 'audio'),
    ('application/pdf', 'report.pdf', 'image'),
  ]) {
    testWidgets('uses $kind shape for $name without loading media', (
      tester,
    ) async {
      final url = 'https://example.com/$name';
      await tester.pumpWidget(
        MaterialApp(
          theme: AppTheme.light(),
          home: Scaffold(
            body: Center(
              child: SizedBox(
                width: 240,
                child: MessageSkeletonBody(
                  content: '![attachment]($url)',
                  tags: [
                    [
                      'imeta',
                      'url $url',
                      'm $mime',
                      'filename $name',
                      'dim 1200x2400',
                    ],
                  ],
                ),
              ),
            ),
          ),
        ),
      );
      final shape = find.byKey(ValueKey('message-skeleton-$kind:$url'));
      expect(shape, findsOneWidget);
      expect(find.byType(Image), findsNothing);
      expect(tester.getSize(shape).width, lessThanOrEqualTo(240));
      if (kind == 'image') {
        expect(tester.getSize(shape), const Size(120, 240));
      } else if (kind == 'audio') {
        expect(tester.getSize(shape).height, 64);
      }
      expect(tester.takeException(), isNull);
    });
  }

  for (final count in [10, 6000]) {
    testWidgets('caps skeleton widgets for $count embeds', (tester) async {
      final content = List.generate(
        count,
        (i) => '![photo](https://example.com/$i.jpg) between',
      ).join('\n');
      await tester.pumpWidget(
        MaterialApp(
          home: Scaffold(
            body: SingleChildScrollView(
              child: MessageSkeletonBody(content: content, tags: const []),
            ),
          ),
        ),
      );
      expect(
        find.byWidgetPredicate(
          (widget) =>
              widget.key is ValueKey<String> &&
              (widget.key! as ValueKey<String>).value.startsWith(
                'message-skeleton-image:',
              ),
        ),
        count == 6000 ? findsNothing : findsNWidgets(4),
      );
      expect(
        find.byKey(const ValueKey('message-skeleton-overflow')),
        findsOneWidget,
      );
      expect(find.byType(SkeletonBar).evaluate().length, lessThanOrEqualTo(9));
      expect(tester.takeException(), isNull);
    });
  }

  for (final content in [
    '`![photo](https://example.com/a.jpg)${' ' * 8192}`',
    '```\n![photo](https://example.com/a.jpg)${' ' * 8192}\n```',
    '${' ' * 8170}![photo](https://example.com/a.jpg)',
  ]) {
    testWidgets(
      'fails closed for cutoff inside Markdown: ${content.substring(0, 3)}',
      (tester) async {
        await tester.pumpWidget(
          MaterialApp(
            home: Scaffold(
              body: MessageSkeletonBody(content: content, tags: const []),
            ),
          ),
        );
        expect(
          find.byKey(const ValueKey('message-skeleton-overflow')),
          findsOneWidget,
        );
        expect(find.byType(SkeletonBar), findsOneWidget);
        expect(
          find.byKey(
            const ValueKey('message-skeleton-image:https://example.com/a.jpg'),
          ),
          findsNothing,
        );
        expect(
          find.byKey(const ValueKey('message-skeleton-gallery')),
          findsNothing,
        );
      },
    );
  }

  testWidgets('does not inspect attachments beyond its fixed input budget', (
    tester,
  ) async {
    await tester.pumpWidget(
      MaterialApp(
        home: Scaffold(
          body: MessageSkeletonBody(
            content: '${'a' * 8192}![photo](https://example.com/tail.jpg)',
            tags: const [],
          ),
        ),
      ),
    );
    expect(
      find.byKey(
        const ValueKey('message-skeleton-image:https://example.com/tail.jpg'),
      ),
      findsNothing,
    );
    expect(
      find.byKey(const ValueKey('message-skeleton-overflow')),
      findsOneWidget,
    );
    expect(find.byType(SkeletonBar).evaluate().length, lessThanOrEqualTo(5));
  });

  for (final content in [
    '[photo](https://example.com/a.jpg)',
    '`![photo](https://example.com/a.jpg)`',
    '```\n![photo](https://example.com/a.jpg)\n```',
    'https://example.com/a.jpg',
  ]) {
    testWidgets('keeps inline or code media text inline: $content', (
      tester,
    ) async {
      await tester.pumpWidget(
        MaterialApp(
          home: Scaffold(
            body: MessageSkeletonBody(content: content, tags: const []),
          ),
        ),
      );
      expect(
        find.byKey(
          const ValueKey('message-skeleton-image:https://example.com/a.jpg'),
        ),
        findsNothing,
      );
      expect(find.byType(SkeletonBar), findsWidgets);
    });
  }

  testWidgets('includes both caption and multiple attachment shapes', (
    tester,
  ) async {
    await tester.pumpWidget(
      MaterialApp(
        theme: AppTheme.light(),
        home: const Scaffold(
          body: SizedBox(
            width: 280,
            child: MessageSkeletonBody(
              content:
                  'Caption\n![photo](https://example.com/a.jpg)\n![voice](https://example.com/b.m4a)',
              tags: [],
            ),
          ),
        ),
      ),
    );
    expect(
      find.byKey(
        const ValueKey('message-skeleton-image:https://example.com/a.jpg'),
      ),
      findsOneWidget,
    );
    expect(
      find.byKey(
        const ValueKey('message-skeleton-audio:https://example.com/b.m4a'),
      ),
      findsOneWidget,
    );
    expect(find.byType(SkeletonBar), findsWidgets);
    expect(tester.takeException(), isNull);
  });
}
