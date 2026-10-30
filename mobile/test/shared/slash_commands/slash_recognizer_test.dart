import 'package:buzz/shared/slash_commands/slash_recognizer.dart';
import 'package:flutter_test/flutter_test.dart';

const _names = {
  'away': 'away',
  'status': 'status',
  'remove': 'remove',
  'kick': 'remove',
  'shrug': 'shrug',
};

({String command, String args})? _recognize(String draft) {
  final found = recognizeSlashDraft<String>(draft, (name) => _names[name]);
  return found == null ? null : (command: found.command, args: found.args);
}

void main() {
  group('recognizeSlashDraft', () {
    const cases = <(String, String, ({String command, String args})?)>[
      ('a bare command', '/away', (command: 'away', args: '')),
      ('upper case', '/AWAY', (command: 'away', args: '')),
      ('mixed case', '/Status  busy', (command: 'status', args: 'busy')),
      ('leading whitespace', '  \n/away', (command: 'away', args: '')),
      ('trailing whitespace', '/away  ', (command: 'away', args: '')),
      ('an alias', '/kick @Eva', (command: 'remove', args: '@Eva')),
      ('an upper case alias', '/KICK', (command: 'remove', args: '')),
      (
        'args keep inner spacing',
        '/status :tada:  all  hands',
        (command: 'status', args: ':tada:  all  hands'),
      ),
      (
        'args may span lines',
        '/shrug one\ntwo',
        (command: 'shrug', args: 'one\ntwo'),
      ),
      ('a command after a mention', '@Eva /status', null),
      ('a mention alone', '@Eva', null),
      ('an unknown command', '/review hello', null),
      ('a path', '/usr/bin/foo', null),
      ('a doubled slash', '//away', null),
      ('a longer name', '/away-now', null),
      ('a name prefix', '/awa', null),
      ('a lone slash', '/', null),
      ('a slash then space', '/ away', null),
      ('a command not first', 'please /away', null),
      ('prose', 'hello', null),
      ('empty', '', null),
      ('whitespace only', '   ', null),
    ];
    for (final (label, draft, expected) in cases) {
      test(label, () => expect(_recognize(draft), expected));
    }
  });

  group('findSlashQuery', () {
    const cases = <(String, String, int?, String?)>[
      ('just a slash', '/', null, ''),
      ('a partial name', '/sh', null, 'sh'),
      ('caret mid-token', '/shrug', 3, 'shrug'),
      ('caret at the end of the token', '/shrug', 6, 'shrug'),
      ('upper case is kept', '/SH', null, 'SH'),
      ('leading whitespace', ' \n /sh', null, 'sh'),
      ('caret before the slash', '/sh', 0, null),
      ('caret past a space', '/shrug ', null, null),
      ('caret in the arguments', '/shrug hi', null, null),
      ('caret back in the token', '/shrug hi', 3, 'shrug'),
      ('text before the slash', 'hi /sh', null, null),
      ('a mention first', '@Eva /sh', null, null),
      ('a doubled slash', '//a', null, null),
      ('a path', '/usr/bin', null, null),
      ('a non-name character', '/a.b', null, null),
      ('no slash', 'shrug', null, null),
      ('empty', '', null, null),
    ];
    for (final (label, text, caret, query) in cases) {
      test(label, () {
        final open = findSlashQuery(text, caret ?? text.length);
        expect(open?.query, query);
        if (open != null) {
          expect(text.substring(open.start, open.start + 1), '/');
          expect(open.end, lessThanOrEqualTo(text.length));
        }
      });
    }

    test('reports the token bounds', () {
      expect(findSlashQuery('  /sh hi', 5), (start: 2, end: 5, query: 'sh'));
    });
  });
}
