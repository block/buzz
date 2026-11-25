import '../../features/channels/message_media.dart';

/// The name a Solo chat is created with, from its first [message] as it is
/// published and that event's [tags].
///
/// 1. The lines the composer appended for attachments are removed: the
///    trailing `![…](address)` and `[…](address)` lines whose address is an
///    `imeta` tag's.
/// 2. Nothing left, and attachments: `Image`, `Video` or `Attachment` for
///    one, `2 images`, `2 videos` or `3 attachments` for several. A kind is
///    its tag's type, or its address's file extension when the type does not
///    settle it, as the timeline draws it.
/// 3. Only web addresses left, plain or `<…>`: `Link to example.com` for one,
///    `2 links` for several, and `Link` when the site cannot be read.
/// 4. Otherwise the text as written, cut to 80 characters, each run of
///    whitespace and control characters flattened to one space. Markdown is
///    not read. A leading `buzz://` source link or block quote, which Start
///    Solo chat seeds, is skipped when a prompt follows it.
/// 5. Nothing to name: `Chat`.
///
/// Desktop renames a chat with its agent harness's title while the stored
/// name still equals the `name` tag of the chat's create event, so this rule
/// need not match anything desktop derives.
///
/// Never throws, so a send cannot fail because of the name, and is linear in
/// the message's length: it runs on the UI isolate in the Send tap.
String startingMeChatTitle(
  String message, {
  List<List<String>> tags = const [],
}) {
  try {
    final sent = parseImetaTags(tags);
    final lines = message.split('\n');
    while (lines.isNotEmpty && sent.containsKey(_attachedAddress(lines.last))) {
      lines.removeLast();
    }
    final text = _prompt(lines.join('\n')).replaceAll(_blank, ' ').trim();
    final words = text.split(' ');
    final title = _cut(
      words.every(_webAddress.hasMatch) ? _linkTitle(words) : text,
    );
    if (title.isNotEmpty) return title;
    return sent.isEmpty ? 'Chat' : _attachmentTitle(sent);
  } catch (_) {
    return 'Chat';
  }
}

/// The address of a line shaped as the composer's attachment lines are:
/// `![image](address)` or `[file name](address)`. Null for any other line.
String? _attachedAddress(String line) {
  if (!line.endsWith(')') || !line.startsWith('[') && !line.startsWith('![')) {
    return null;
  }
  final label = line.lastIndexOf('](');
  return label < 0 ? null : line.substring(label + 2, line.length - 1);
}

/// What [text] asks, without the source Start Solo chat seeds before it: a
/// `buzz://` link, or a block quote. A quote with no prompt after it is read
/// as the prompt.
String _prompt(String text) {
  final lines = text.trim().replaceFirst(_sourceLinks, '').split('\n');
  final prompt = lines.skipWhile((line) => line.startsWith('>')).join('\n');
  return prompt.trim().isNotEmpty
      ? prompt.trim()
      : lines
            .map((line) => line.replaceFirst(_quoteMark, ''))
            .join('\n')
            .trim();
}

/// A run of whitespace and C0 and C1 control characters. The relay trims
/// U+0085 from a name as whitespace, which `\s` does not match, so a name
/// that kept one could be empty to the relay and refused on every retry.
final _blank = RegExp(r'[\s\x00-\x1f\x7f-\x9f]+');
final _sourceLinks = RegExp(r'^(?:buzz://\S*\s*)+', caseSensitive: false);
final _quoteMark = RegExp(r'^>\s?');

/// A channel name's length and edges: 80 characters, no leading `#`.
String _cut(String text) => String.fromCharCodes(
  text.runes.take(80),
).trim().replaceFirst(RegExp(r'^[#\s]+'), '');

final _webAddress = RegExp(
  r'^(?:https?://[^\s<>]+|<https?://[^\s<>]+>)$',
  caseSensitive: false,
);

/// A name for a message that is only web [addresses], which read as noise:
/// the site of one, or how many.
String _linkTitle(List<String> addresses) {
  if (addresses.length > 1) return '${addresses.length} links';
  try {
    final address = addresses.single.replaceAll(RegExp('[<>]'), '');
    final host = Uri.decodeComponent(Uri.parse(address).host);
    // An IPv6 address reads as one in its brackets.
    final site = host.contains(':')
        ? '[$host]'
        : host.replaceFirst(RegExp(r'^www\.'), '');
    return site.isEmpty || site.contains(_control) ? 'Link' : 'Link to $site';
  } catch (_) {
    return 'Link';
  }
}

final _control = RegExp(r'[\x00-\x1f\x7f-\x9f]');

/// A plain description of the attachments [sent]: `Image`, `2 videos`,
/// `3 attachments`. Audio, a voice note and any other file are attachments.
String _attachmentTitle(Map<String, ImetaEntry> sent) {
  final kinds = {
    for (final MapEntry(key: url, value: imeta) in sent.entries)
      classifyMediaUrl(url, imeta: imeta),
  };
  final noun = switch (kinds.length == 1 ? kinds.single : null) {
    MessageMediaKind.image => 'image',
    MessageMediaKind.video => 'video',
    _ => 'attachment',
  };
  return sent.length == 1
      ? '${noun[0].toUpperCase()}${noun.substring(1)}'
      : '${sent.length} ${noun}s';
}
