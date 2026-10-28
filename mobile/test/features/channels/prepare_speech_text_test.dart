import 'package:buzz/features/channels/read_aloud/prepare_speech_text.dart';
import 'package:flutter_test/flutter_test.dart';

void main() {
  test('strips fenced code', () {
    final out = prepareSpeechText(
      'Here is some code:\n```rust\nfn main() {}\n```\nDone.',
    );
    expect(out, contains('code block omitted'));
    expect(out, isNot(contains('fn main')));
  });

  test('strips tilde fences', () {
    final out = prepareSpeechText(
      'Here:\n~~~python\nprint(\'hi\')\n~~~\nDone.',
    );
    expect(out, contains('code block omitted'));
    expect(out, isNot(contains('print')));
  });

  test('strips inline code', () {
    expect(prepareSpeechText('Call `foo()` now.'), 'Call foo() now.');
  });

  test('strips urls and keeps sentence punctuation', () {
    expect(prepareSpeechText('See https://x.y/z.'), 'See link omitted.');
    expect(
      prepareSpeechText('Visit https://example.com!'),
      'Visit link omitted!',
    );
    expect(
      prepareSpeechText('Did you see https://example.com?'),
      'Did you see link omitted?',
    );
    expect(
      prepareSpeechText('Check https://example.com for more info.'),
      'Check link omitted for more info.',
    );
  });

  test('strips markdown and keeps snake case', () {
    expect(
      prepareSpeechText('**bold** and *italic* and _under_'),
      'bold and italic and under',
    );
    final identifiers = prepareSpeechText('call foo_bar() or baz_qux');
    expect(identifiers, contains('foo_bar'));
    expect(identifiers, contains('baz_qux'));
  });

  test('expands integers and times', () {
    expect(prepareSpeechText('42'), 'forty two');
    expect(prepareSpeechText('0'), 'zero');
    expect(prepareSpeechText('11'), 'eleven');
    expect(prepareSpeechText('100'), 'one hundred');
    expect(prepareSpeechText('1000'), 'one thousand');
    expect(prepareSpeechText('1234'), 'one thousand two hundred thirty four');
    expect(prepareSpeechText('11:30'), 'eleven thirty');
    expect(prepareSpeechText('9:00'), 'nine');
    expect(prepareSpeechText('9:05'), 'nine oh five');
  });

  test('collapses whitespace and drops trivial text', () {
    expect(prepareSpeechText('  hello   world  '), 'hello world');
    expect(prepareSpeechText('.'), isEmpty);
    expect(prepareSpeechText(','), isEmpty);
    expect(prepareSpeechText('ok'), 'ok');
  });

  test('drops imeta attachment lines and spacer artifacts', () {
    const url = 'https://cdn.example/voice.m4a';
    final out = prepareSpeechText(
      'Listen.\n[voice]($url)\n||\n\n||\nDone.',
      tags: [
        ['imeta', 'url $url', 'm audio/mp4'],
      ],
    );
    expect(out, 'Listen. Done.');
    expect(out, isNot(contains(url)));
  });

  test('splits on a sentence boundary inside the platform limit', () {
    expect(splitSpeechChunks('Hello there. Second sentence.', maxLength: 16), [
      'Hello there.',
      'Second sentence.',
    ]);
  });
}
