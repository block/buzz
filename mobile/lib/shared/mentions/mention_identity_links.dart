/// Identity links: a mention written into a message body as a Markdown link
/// to its exact public key, `[@Honey](nostr:npub1…)`.
///
/// The link names a person for display only. It carries no `p` tag, so it
/// notifies nobody, and it needs no tag for a reader to resolve it. An edit
/// writes its mentions this way, as buzz-app's `profileMentionLink` does, so
/// both clients read each other's edited messages.
library;

import '../utils/string_utils.dart';

final _asciiPunctuation = RegExp(r'[!-/:-?\[-`{-~]');
final _escapedPunctuation = RegExp(r'\\([!-/:-@\[-`{-~])');
final _lineBreaks = RegExp(r'[\r\n]+');
final _target = RegExp(r'^nostr:npub1[02-9ac-hj-np-z]+$');
final _link = RegExp(
  r'(?<![!\\])\[((?:\\[!-~]|[^\[\]\\\n])+)\]\((nostr:npub1[02-9ac-hj-np-z]+)\)',
);

/// The identity link for [label] (`@Name`) naming [pubkey]: a one-line
/// label with every ASCII punctuation mark escaped, so a name cannot author
/// emphasis, code or a link, on a `nostr:npub1…` target. [label] alone when
/// [pubkey] is not a public key.
String mentionIdentityLink(String label, String pubkey) {
  final npub = fullNpub(pubkey);
  if (npub == null) return label;
  final text = label
      .replaceAll(_lineBreaks, ' ')
      .replaceAllMapped(_asciiPunctuation, (match) => '\\${match[0]}');
  return '[$text](nostr:$npub)';
}

/// The lowercase hex key a link destination names, or null unless [target]
/// is exactly a lowercase `nostr:npub1…` with a valid checksum.
String? mentionIdentityLinkKey(String target) =>
    _target.hasMatch(target) ? typedPublicKey(target) : null;

/// [name], a mention name for an identity link to [pubkey], or null when it
/// is only the compact npub a nameless tagged agent's mention name falls back
/// to, so the link keeps its written label instead.
String? mentionIdentityLinkName(String pubkey, String? name) =>
    name == null || name == shortPubkey(pubkey) ? null : name;

/// Whether [label], a link's visible text, reads as a mention.
bool isMentionIdentityLabel(String label) =>
    label.length > 1 && label.startsWith('@');

/// Every identity link in [text], in order: its source range, its label as
/// read (`@Name`, escapes removed) and the key it names. Callers skip the
/// ones inside code.
List<({int start, int end, String label, String pubkey})> mentionIdentityLinks(
  String text,
) => [
  for (final match in _link.allMatches(text))
    if ((
          match[1]!.replaceAllMapped(_escapedPunctuation, (m) => m[1]!),
          mentionIdentityLinkKey(match[2]!),
        )
        case (final label, final pubkey?) when isMentionIdentityLabel(label))
      (start: match.start, end: match.end, label: label, pubkey: pubkey),
];
