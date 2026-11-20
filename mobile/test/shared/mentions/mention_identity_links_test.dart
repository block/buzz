import 'package:buzz/shared/mentions/mention_identity_links.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:nostr/nostr.dart' as nostr;

void main() {
  final key = 'b' * 64;
  final target =
      'nostr:${nostr.Nip19.encode(prefix: nostr.Nip19Prefix.npub, data: key)}';

  // The wire form is buzz-app's `profileMentionLink`: a one-line label with
  // every ASCII punctuation mark but `@` escaped, on a `nostr:npub1…` target.
  test('an identity link round-trips its label and key', () {
    for (final (label, written) in [
      ('@Honey', '@Honey'),
      ('@Bob-bot', r'@Bob\-bot'),
      ('@Bee [x]', r'@Bee \[x\]'),
      ('@A *B*', r'@A \*B\*'),
      ('@A `B`', r'@A \`B\`'),
      ('@O\'Brien (ops)', r"@O\'Brien \(ops\)"),
      ('@Sam\nSmith', '@Sam Smith'),
    ]) {
      final link = mentionIdentityLink(label, key);
      expect(link, '[$written]($target)', reason: label);
      final read = mentionIdentityLinks('see $link.').single;
      expect(read.label, label.replaceAll('\n', ' '), reason: label);
      expect(read.pubkey, key, reason: label);
      expect((read.start, read.end), (4, 4 + link.length), reason: label);
    }
    expect(mentionIdentityLink('@Honey', 'not a key'), '@Honey');
  });

  test('only a mention label on an exact lowercase npub is an identity', () {
    for (final text in [
      '[Honey]($target)',
      '[@]($target)',
      '![@Honey]($target)',
      '\\[@Honey]($target)',
      '[@Honey](${target.toUpperCase()})',
      '[@Honey](${target.substring(0, target.length - 1)}q)',
      '[@Honey](https://example.org/)',
      '[@Honey]($target "title")',
    ]) {
      expect(mentionIdentityLinks(text), isEmpty, reason: text);
    }
    expect(mentionIdentityLinkKey(target), key);
    expect(mentionIdentityLinkKey(target.toUpperCase()), isNull);
  });
}
