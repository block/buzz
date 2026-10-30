import 'package:buzz/shared/slash_commands/slash_tokenizer.dart';
import 'package:flutter_test/flutter_test.dart';

void main() {
  group('tokenizeSlashArgs', () {
    const cases = <(String, String, List<String>)>[
      ('empty', '', []),
      ('whitespace only', ' \t\n', []),
      ('one token', 'me', ['me']),
      ('several tokens', 'me to  stretch', ['me', 'to', 'stretch']),
      ('newlines and tabs', 'a\nb\tc', ['a', 'b', 'c']),
      ('quotes are plain text', '"a b"', ['"a', 'b"']),
      ('mentions and channels', '@Eva #general hi', ['@Eva', '#general', 'hi']),
      ('emoji', ':tada: 🎉 done', [':tada:', '🎉', 'done']),
    ];
    for (final (label, args, tokens) in cases) {
      test(label, () {
        expect([for (final t in tokenizeSlashArgs(args)) t.text], tokens);
      });
    }

    test('tokens carry their offsets', () {
      const args = ' ab  cd';
      expect(tokenizeSlashArgs(args), const [
        SlashToken('ab', 1, 3),
        SlashToken('cd', 5, 7),
      ]);
    });
  });

  group('slashArgsFrom', () {
    const cases = <(String, String, int, String)>[
      ('from the start', 'a  b c', 0, 'a  b c'),
      ('keeps inner spacing', 'a  b   c', 1, 'b   c'),
      ('skips leading space', '  a b', 0, 'a b'),
      ('drops trailing space', 'a b  ', 1, 'b'),
      ('keeps newlines', 'a b\nc', 1, 'b\nc'),
      ('past the end', 'a b', 2, ''),
      ('empty', '', 0, ''),
    ];
    for (final (label, args, index, rest) in cases) {
      test(label, () => expect(slashArgsFrom(args, index), rest));
    }
  });
}
