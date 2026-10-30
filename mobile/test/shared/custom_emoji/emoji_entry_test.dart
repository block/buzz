import 'package:buzz/shared/custom_emoji/emoji_entry.dart';
import 'package:buzz/shared/relay/relay.dart';
import 'package:flutter_test/flutter_test.dart';

NostrEvent _event({
  String id = 'e1',
  String pubkey = 'a',
  int createdAt = 1,
  int kind = EventKind.emojiEntry,
  List<List<String>>? tags,
  String d = 'parrot',
  String url = 'https://cdn.test/parrot.png',
}) => NostrEvent(
  id: id,
  pubkey: pubkey,
  createdAt: createdAt,
  kind: kind,
  tags:
      tags ??
      [
        ['d', d],
        ['emoji', d, url],
      ],
  content: d,
  sig: '',
);

void main() {
  test('only a well-formed entry becomes an emoji', () {
    final invalid = <String, NostrEvent>{
      'wrong kind': _event(kind: 30030),
      'no d tag': _event(
        tags: [
          ['emoji', 'parrot', 'https://cdn.test/p.png'],
        ],
      ),
      'two d tags': _event(
        tags: [
          ['d', 'parrot'],
          ['d', 'parrot'],
          ['emoji', 'parrot', 'https://cdn.test/p.png'],
        ],
      ),
      'two emoji tags': _event(
        tags: [
          ['d', 'parrot'],
          ['emoji', 'parrot', 'https://cdn.test/p.png'],
          ['emoji', 'parrot', 'https://cdn.test/q.png'],
        ],
      ),
      'd not normalized': _event(d: 'Parrot'),
      'emoji tag names another shortcode': _event(
        tags: [
          ['d', 'parrot'],
          ['emoji', 'other', 'https://cdn.test/p.png'],
        ],
      ),
      'javascript url': _event(url: 'javascript:alert(1)'),
      'relative url': _event(url: '/media/p.png'),
      'file url': _event(url: 'file:///etc/passwd'),
    };
    for (final entry in invalid.entries) {
      expect(emojiFromEntryEvent(entry.value), isNull, reason: entry.key);
    }
    expect(emojiFromEntryEvent(_event())?.shortcode, 'parrot');
    expect(
      emojiFromEntryEvent(_event(url: 'http://cdn.test/p.png'))?.url,
      'http://cdn.test/p.png',
    );
  });

  test('the newest event wins and a second author is reported', () {
    final resolution = resolveEmojiEntries([
      _event(id: 'old', createdAt: 1, url: 'https://cdn.test/old.png'),
      _event(id: 'new', createdAt: 5, url: 'https://cdn.test/new.png'),
      _event(
        id: 'other',
        pubkey: 'b',
        createdAt: 3,
        d: 'parrot',
        url: 'https://cdn.test/other.png',
      ),
      _event(id: 'solo', d: 'solo'),
    ]);
    expect(resolution.emoji.map((e) => '${e.shortcode} ${e.url}'), [
      'parrot https://cdn.test/new.png',
      'solo https://cdn.test/solo.png',
    ]);
    expect(resolution.conflicts, ['parrot']);

    final reordered = resolveEmojiEntries([
      _event(id: 'b', pubkey: 'b', url: 'https://cdn.test/b.png'),
      _event(id: 'a', pubkey: 'a', url: 'https://cdn.test/a.png'),
    ]);
    expect(reordered.emoji.single.url, 'https://cdn.test/a.png');
  });

  test('results are capped at 50', () {
    final resolution = resolveEmojiEntries([
      for (var i = 0; i < 80; i++) _event(id: 'e$i', d: 'emoji$i'),
    ]);
    expect(resolution.emoji, hasLength(maxEmojiCatalogResults));
  });

  test('search text follows the entry content', () {
    expect(emojiCatalogSearchText(':Party_Par-'), 'party par');
    expect(emojiCatalogSearchText('p'), isNull);
    expect(emojiCatalogSearchText(' _ '), isNull);
  });
}
