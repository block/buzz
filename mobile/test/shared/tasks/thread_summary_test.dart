import 'package:buzz/shared/tasks/thread_summary.dart';
import 'package:flutter_test/flutter_test.dart';

ThreadMessageDigest msg(String author, String text) =>
    ThreadMessageDigest(author: author, text: text);

void main() {
  group('summarizeThread', () {
    test('says so rather than returning an empty string', () {
      expect(summarizeThread(const []), 'No messages to summarize yet.');
      expect(
        summarizeThread([msg('Ada', '   '), msg('Grace', '\n\t')]),
        'No messages to summarize yet.',
      );
    });

    test('headlines the message count and participants', () {
      final summary = summarizeThread([
        msg('Ada', 'The relay drops the connection after an hour.'),
        msg('Grace', 'We should add a keepalive ping.'),
        msg('Ada', 'Agreed, I will take it.'),
      ]);
      expect(summary, startsWith('## Thread summary\n'));
      expect(summary, contains('_3 messages from Ada and Grace._'));
    });

    test('uses the singular for a one-message thread', () {
      expect(
        summarizeThread([msg('Ada', 'Deploy is done.')]),
        contains('_1 message from Ada._'),
      );
    });

    test('collapses a long participant roster', () {
      final summary = summarizeThread([
        for (final name in ['A', 'B', 'C', 'D', 'E']) msg(name, 'a note here'),
      ]);
      expect(summary, contains('A, B, C and 2 others'));
    });

    test('routes questions to their own section, not to highlights', () {
      final summary = summarizeThread([
        msg('Ada', 'We decided to ship behind a flag.'),
        msg('Grace', 'Who owns the rollback plan?'),
      ]);
      final highlights = summary.indexOf('**Highlights**');
      final questions = summary.indexOf('**Open questions**');
      expect(highlights, greaterThan(-1));
      expect(questions, greaterThan(highlights));
      expect(
        summary.substring(highlights, questions),
        contains('**Ada:** We decided to ship behind a flag.'),
      );
      expect(
        summary.substring(questions),
        contains('**Grace:** Who owns the rollback plan?'),
      );
    });

    test('omits sections that have no content', () {
      final onlyQuestions = summarizeThread([msg('Ada', 'Is it live yet?')]);
      expect(onlyQuestions, isNot(contains('**Highlights**')));
      expect(onlyQuestions, contains('**Open questions**'));

      final onlyStatements = summarizeThread([msg('Ada', 'It is live.')]);
      expect(onlyStatements, contains('**Highlights**'));
      expect(onlyStatements, isNot(contains('**Open questions**')));
      expect(onlyStatements, isNot(contains('**Links**')));
    });

    test('prefers decision-bearing lines over filler', () {
      final summary = summarizeThread([
        msg('Ada', 'morning'),
        msg('Grace', 'hey'),
        msg('Ada', 'We decided to revert the migration; I will own it.'),
        msg('Grace', 'ok'),
      ], maxHighlights: 1);
      expect(
        summary,
        contains('**Ada:** We decided to revert the migration; I will own it.'),
      );
      expect(summary, isNot(contains('**Grace:** hey')));
    });

    test('keeps selected highlights in thread order', () {
      // Relevance picks which lines survive; chronology decides how they read.
      final summary = summarizeThread([
        msg('Ada', 'The plan is to cut a release candidate on Friday.'),
        msg('Grace', 'Blocked on the signing cert until Thursday.'),
      ], maxHighlights: 2);
      expect(
        summary.indexOf('**Ada:**'),
        lessThan(summary.indexOf('**Grace:**')),
      );
    });

    test(
      'collects links once, in first-seen order, without trailing punctuation',
      () {
        final summary = summarizeThread([
          msg('Ada', 'See https://example.com/pr/1.'),
          msg(
            'Grace',
            'Also https://example.com/pr/2 and https://example.com/pr/1',
          ),
        ]);
        final links = summary.substring(summary.indexOf('**Links**'));
        expect(links, contains('- https://example.com/pr/1\n'));
        expect(links, contains('- https://example.com/pr/2'));
        expect(
          'https://example.com/pr/1'.allMatches(links).length,
          1,
          reason: 'a repeated link must not be listed twice',
        );
      },
    );

    test('replaces code with a placeholder so bullets stay well-formed', () {
      final summary = summarizeThread([
        msg('Ada', 'Run this to reproduce:\n```\njust relay\n```\nthen retry.'),
      ]);
      expect(summary, contains('[code]'));
      expect(summary, isNot(contains('just relay')));
      expect(summary.split('\n').where((l) => l == '```'), isEmpty);
    });

    test('flattens newlines so one message stays one bullet', () {
      final summary = summarizeThread([
        msg('Ada', 'first line\nsecond line\n\nthird line'),
      ]);
      expect(summary, contains('**Ada:** first line second line third line'));
    });

    test('names a blank author rather than emitting an empty label', () {
      expect(summarizeThread([msg('  ', 'a note')]), contains('**Unknown:**'));
    });

    test('is deterministic for the same input', () {
      final messages = [
        msg('Ada', 'We should ship behind a flag.'),
        msg('Grace', 'Who owns the rollback?'),
        msg('Ada', 'Blocked on https://example.com/cert'),
      ];
      expect(summarizeThread(messages), summarizeThread(messages));
    });

    test('honours the section caps', () {
      final messages = [
        for (var i = 0; i < 12; i++)
          msg('Ada', 'We decided on option $i for the rollout plan.'),
        for (var i = 0; i < 12; i++) msg('Grace', 'What about option $i?'),
      ];
      final summary = summarizeThread(
        messages,
        maxHighlights: 2,
        maxOpenQuestions: 1,
      );
      expect(
        summary.split('\n').where((l) => l.contains('**Ada:**')).length,
        2,
      );
      expect(
        summary.split('\n').where((l) => l.contains('**Grace:**')).length,
        1,
      );
    });

    test('elides an over-long line at the requested width', () {
      final summary = summarizeThread([
        msg('Ada', 'word ' * 200),
      ], maxCharsPerLine: 40);
      final bullet = summary
          .split('\n')
          .firstWhere((line) => line.startsWith('- **Ada:**'));
      expect(bullet, endsWith('…'));
      // '- **Ada:** ' is chrome around the 40-character line itself.
      expect(bullet.length, lessThanOrEqualTo('- **Ada:** '.length + 40));
    });
  });

  group('elideSummaryLine', () {
    test('leaves a short line alone', () {
      expect(elideSummaryLine('short', 40), 'short');
      expect(elideSummaryLine('exactly ten', 11), 'exactly ten');
    });

    test('cuts on a word boundary when one is available', () {
      expect(elideSummaryLine('alpha beta gamma delta', 16), 'alpha beta…');
    });

    test('cuts mid-token rather than collapsing to nothing', () {
      final elided = elideSummaryLine('a ${'z' * 60}', 20);
      expect(elided.length, 20);
      expect(elided, endsWith('…'));
    });
  });

  group('threadTaskTitle', () {
    test('uses the first message with content', () {
      expect(
        threadTaskTitle([
          msg('Ada', '   '),
          msg('Grace', 'Relay drops long-lived connections'),
        ]),
        'Relay drops long-lived connections',
      );
    });

    test('falls back when the thread has no text', () {
      expect(threadTaskTitle(const []), 'Thread summary');
      expect(threadTaskTitle([msg('Ada', '\n')]), 'Thread summary');
    });

    test('stays well inside the relay title ceiling', () {
      final title = threadTaskTitle([msg('Ada', 'word ' * 200)]);
      expect(title.runes.length, lessThanOrEqualTo(200));
      expect(title, endsWith('…'));
    });
  });
}
