/// The name a Solo chat is created with, from its first [message] as it is
/// published and that event's [tags].
///
/// This is desktop's rule: `startingChatTitle` in
/// `src/features/solo/chat-title.ts` of block/buzz-app. Desktop gives a chat
/// its agent harness's title only while the stored name still equals the
/// title it derives from the published first message and its `imeta` tags, so
/// the two must agree. `me_chat_creation_test.dart` carries desktop's cases.
///
/// - Text is the title, cut to 80 characters. A leading `buzz://` source link
///   is left out, a link reads as its label, and media and links to attached
///   files are left out.
/// - A message that is only web addresses is `Link to example.com`, or
///   `2 links`.
/// - A message that is only media is `Image`, `Video` or `Attachment`, or a
///   count: `2 images`, `2 videos`, `3 attachments`.
/// - Anything else is `Chat`.
///
/// Known differences, where desktop will not rename the chat:
///
/// - Mobile skips a leading block quote, which Start Solo chat seeds for a
///   message the agent can't open. Desktop keeps the quote in the title.
/// - Desktop leaves out a leading `[Chat: …](buzz://channel/…)` link, which
///   its own composer writes, and a leading `@Name` of the tagged agent.
/// - Desktop parses Markdown in full. This reads what a phone composer sends
///   and a person types, so links in an indented code block or an HTML block,
///   uncommon named entities and non-ASCII site names can read differently.
String startingMeChatTitle(
  String message, {
  List<List<String>> tags = const [],
}) {
  final sent = _sentAttachments(tags);
  // Media shows beside the text: every image, and each link to a sent file.
  final embedded = <String>{};
  final text = StringBuffer();
  final source = message.trim();
  var offset = 0;
  for (final link in _links(source)) {
    final url = link.url == null ? null : _mediaUrl(link.url!);
    final attached = url != null && sent.containsKey(url);
    if (link.start < offset || !link.isImage && !attached) continue;
    if (url != null && !attached) embedded.add(url);
    text.write(source.substring(offset, link.start));
    offset = link.end;
  }
  text.write(source.substring(offset));
  var body = text.toString().trim();
  // Source links are context rather than a useful chat title.
  while (_buzzLink.hasMatch(body)) {
    final end = body.indexOf(RegExp(r'\s'));
    body = end < 0 ? '' : body.substring(end).trimLeft();
  }
  // A quoted source is context too; title from the prompt after it, else
  // from the quote itself.
  final lines = body.split('\n');
  final prompt = lines.skipWhile((line) => line.startsWith('>')).join('\n');
  body = prompt.trim().isNotEmpty
      ? prompt.trim()
      : lines.map((line) => line.replaceFirst(RegExp(r'^>\s?'), '')).join('\n');
  final flat = _withLinkLabels(body).replaceAll(RegExp(r'\s+'), ' ');
  final title = _cut(flat);
  // A message that is only media or web links is titled by what it sent.
  final described =
      _linkTitle(flat) ??
      (title.isNotEmpty
          ? title
          : _attachmentSummary([
              ...sent.values,
              for (final url in embedded)
                _videoAddress.hasMatch(url) ? _Sent.video : _Sent.image,
            ]));
  final name = _cut(described);
  return name.isEmpty ? 'Chat' : name;
}

final _buzzLink = RegExp('^buzz:', caseSensitive: false);

/// A channel name's length and edges: 80 characters, no leading `#`.
String _cut(String text) => String.fromCharCodes(
  text.runes.take(80),
).trim().replaceFirst(RegExp(r'^[#\s]+'), '');

/// What an attachment is, as far as a title words it.
enum _Sent { image, video, other }

/// The attachments [tags] describe, by address, as desktop's
/// `parseAttachments` and `attachmentKind` read `imeta` tags.
Map<String, _Sent> _sentAttachments(List<List<String>> tags) {
  final sent = <String, _Sent>{};
  for (final tag in tags) {
    if (tag.isEmpty || tag.first != 'imeta') continue;
    final fields = {
      for (final field in tag.skip(1))
        if (field.indexOf(' ') case final split when split > 0)
          field.substring(0, split): field.substring(split + 1),
    };
    final url = _mediaUrl(fields['url'] ?? '');
    if (url == null || sent.containsKey(url)) continue;
    sent[url] = _kind(fields, url);
  }
  return sent;
}

_Sent _kind(Map<String, String> fields, String url) {
  final mime = fields['m']?.toLowerCase() ?? '';
  final name = (fields['filename'] ?? Uri.parse(url).pathSegments.lastOrNull)
      ?.toLowerCase();
  if (mime.startsWith('image/')) return _Sent.image;
  // A voice note from an older build is an MP4 with an audio track.
  final voiceNote =
      (mime == 'video/mp4' || mime.startsWith('video/mp4;')) &&
      name != null &&
      name.startsWith('voice-note-') &&
      name.endsWith('.mp4');
  if (mime.startsWith('video/') && !voiceNote) return _Sent.video;
  if (mime.isNotEmpty) return _Sent.other;
  if (_videoAddress.hasMatch(url)) return _Sent.video;
  return _imageAddress.hasMatch(url) ? _Sent.image : _Sent.other;
}

final _videoAddress = RegExp(r'\.(mp4|webm)(?:\?|$)', caseSensitive: false);
final _imageAddress = RegExp(
  r'\.(png|jpe?g|gif|webp|avif)(?:\?|$)',
  caseSensitive: false,
);

/// [value] as an attachment's address, which is HTTPS without credentials.
/// Null for any other.
String? _mediaUrl(String value) {
  final uri = Uri.tryParse(value);
  return uri != null &&
          uri.scheme == 'https' &&
          uri.host.isNotEmpty &&
          uri.userInfo.isEmpty
      ? uri.toString()
      : null;
}

/// A plain description of attachments: `Image`, `2 videos`, `3 attachments`.
/// Empty when there are none.
String _attachmentSummary(List<_Sent> sent) {
  if (sent.isEmpty) return '';
  final noun = switch (sent.toSet().length == 1 ? sent.first : _Sent.other) {
    _Sent.image => 'image',
    _Sent.video => 'video',
    _Sent.other => 'attachment',
  };
  return sent.length == 1
      ? '${noun[0].toUpperCase()}${noun.substring(1)}'
      : '${sent.length} ${noun}s';
}

/// A title for [text] that is only web addresses, which read as noise: the
/// site of one, or how many. Null when the text says anything else.
String? _linkTitle(String text) {
  final links = text.split(RegExp(r'\s+')).where((link) => link.isNotEmpty);
  if (links.isEmpty) return null;
  final urls = [for (final link in links) _webAddress.firstMatch(link)?[1]];
  if (urls.contains(null)) return null;
  if (urls.length > 1) return '${urls.length} links';
  final host = Uri.decodeComponent(Uri.tryParse(urls.single!)?.host ?? '');
  // An IPv6 address keeps its brackets, as desktop's `URL.hostname` does.
  final site = host.contains(':') ? '[$host]' : host;
  return site.isEmpty
      ? 'Link'
      : 'Link to ${site.replaceFirst(RegExp(r'^www\.'), '')}';
}

final _webAddress = RegExp(r'^<?(https?://[^\s<>]+)>?$', caseSensitive: false);

/// [text] with each link replaced by its label. Code and everything else
/// stay as written.
String _withLinkLabels(String text) {
  final shown = StringBuffer();
  var offset = 0;
  for (final link in _links(text)) {
    if (link.isImage) continue;
    shown
      ..write(text.substring(offset, link.start))
      ..write(link.label);
    offset = link.end;
  }
  shown.write(text.substring(offset));
  return shown.toString();
}

/// A Markdown link or image in a message: where it is, the text its label
/// shows, and its destination when it has one.
typedef _Link = ({int start, int end, bool isImage, String label, String? url});

/// The links and images in [text], in order. Reads the Markdown a composer
/// sends and a person types: `[label](url)`, `![alt](url)`, `<url>` and
/// `[label][reference]`. Code is not Markdown, and a backslash escapes.
List<_Link> _links(String text) {
  final code = [
    for (final match in _code.allMatches(text)) (match.start, match.end),
  ];
  final definitions = <String, String>{};
  // A definition starts a block; it never continues a paragraph.
  var start = 0;
  var opensBlock = true;
  for (final line in text.split('\n')) {
    final end = start + line.length;
    final match = opensBlock && !_inCode(code, start, end)
        ? _definition.firstMatch(line)
        : null;
    if (match != null) {
      definitions.putIfAbsent(_reference(match[1]!), () => match[2]!);
      code.add((start, end));
    }
    opensBlock = match != null || line.trim().isEmpty;
    start = end + 1;
  }
  final links = <_Link>[];
  void add(
    int start,
    int end,
    bool isImage,
    String label,
    String? url, {
    bool nested = false,
  }) {
    // An escaped `!` leaves a link; an escaped bracket leaves text.
    if (_escaped(text, start)) {
      if (!isImage) return;
      isImage = false;
      start++;
    }
    if (_inCode(code, start, end) ||
        !nested && links.any((link) => start < link.end && end > link.start)) {
      return;
    }
    links.add((
      start: start,
      end: end,
      isImage: isImage,
      label: label,
      url: url,
    ));
  }

  for (final match in _inlineLink.allMatches(text)) {
    final url = match[1];
    if (url != null) {
      add(match.start, match.end, false, url, url);
      continue;
    }
    final isImage = match[2]!.isNotEmpty;
    add(
      match.start,
      match.end,
      isImage,
      _labelText(match[3]!),
      _unescape(match[4] ?? match[5]!),
    );
    if (isImage) continue;
    // A link's label can hold an image, which is media like any other.
    final label = match.start + 1;
    for (final image in _inlineLink.allMatches(match[3]!)) {
      if (image[2]?.isNotEmpty != true) continue;
      add(
        label + image.start,
        label + image.end,
        true,
        _labelText(image[3]!),
        _unescape(image[4] ?? image[5]!),
        nested: true,
      );
    }
  }
  if (definitions.isNotEmpty) {
    for (final match in _referenceLink.allMatches(text)) {
      final named = match[3];
      final url =
          definitions[_reference(
            named == null || named.isEmpty ? match[2]! : named,
          )];
      if (url == null) continue;
      add(
        match.start,
        match.end,
        match[1]!.isNotEmpty,
        _labelText(match[2]!),
        _unescape(url.replaceFirst(RegExp(r'^<(.*)>$'), r'$1')),
      );
    }
  }
  return links..sort((a, b) => a.start.compareTo(b.start));
}

/// Whether [start] or [end] falls in one of [ranges]. Code within a link's
/// label is part of the link; a link that begins or ends in code is code.
bool _inCode(List<(int, int)> ranges, int start, int end) => ranges.any(
  (range) =>
      range.$1 <= start && start < range.$2 ||
      range.$1 < end && end <= range.$2,
);

/// Whether an odd run of backslashes escapes the character at [index].
bool _escaped(String text, int index) {
  var slashes = 0;
  while (index - slashes > 0 && text[index - slashes - 1] == r'\') {
    slashes++;
  }
  return slashes.isOdd;
}

/// How a reference names its definition: case and spacing don't count.
String _reference(String label) =>
    label.trim().replaceAll(RegExp(r'\s+'), ' ').toLowerCase();

/// The text a link's [label] shows: its words, without emphasis, code marks,
/// escapes or entities.
String _labelText(String label) {
  // Code and escaped characters are literal: hold them clear of emphasis.
  final held = <String>[];
  final marked = label.replaceAllMapped(_literal, (match) {
    held.add(match[2]?.trim() ?? match[3] ?? _entity(match[4]!) ?? match[0]!);
    return '\u0000${held.length - 1}\u0000';
  });
  var shown = marked;
  for (final emphasis in _emphasis) {
    shown = shown.replaceAllMapped(emphasis, (match) => match[1]!);
  }
  return shown.replaceAllMapped(
    RegExp('\u0000(\\d+)\u0000'),
    (match) => held[int.parse(match[1]!)],
  );
}

/// [text] without its backslash escapes and entities, as a destination.
String _unescape(String text) => text.replaceAllMapped(
  _literal,
  (match) => match[3] ?? _entity(match[4] ?? '') ?? match[0]!,
);

/// The character a Markdown entity names, for the few a person writes.
String? _entity(String name) => switch (name.toLowerCase()) {
  'amp' => '&',
  'lt' => '<',
  'gt' => '>',
  'quot' => '"',
  'apos' => "'",
  'nbsp' => ' ',
  final code when code.startsWith('#x') => _character(code.substring(2), 16),
  final code when code.startsWith('#') => _character(code.substring(1), 10),
  _ => null,
};

String? _character(String digits, int radix) {
  final code = int.tryParse(digits, radix: radix);
  return code == null || code <= 0 || code > 0x10ffff
      ? null
      : String.fromCharCode(code);
}

// A link's label: any text, with escapes and one level of brackets.
const _label = r'((?:\\[\s\S]|[^\[\]\\]|\[(?:\\[\s\S]|[^\[\]\\])*\])*)';
// `<url>`, or `[label](url)` and `![alt](url)` with a bare or `<…>` address
// and perhaps a title.
const _address = r'(?:\\[\s\S]|[^\s()<>\\])';
final _inlineLink = RegExp(
  r'<([A-Za-z][A-Za-z0-9+.\-]{1,31}:[^\s<>]*)>'
  '|(!?)\\[$_label\\]'
  r'\(\s*(?:<([^<>\n]*)>'
  '|((?:$_address|\\($_address*\\))*))'
  r"""(?:\s+(?:"[^"]*"|'[^']*'|\([^()]*\)))?\s*\)""",
);
// `[label][reference]`, `[reference][]` and `[reference]`, as link or image.
final _referenceLink = RegExp(
  '(!?)\\[$_label\\]'
  r'(?:\[((?:\\[\s\S]|[^\[\]\\])*)\])?(?![(\[])',
);
final _definition = RegExp(
  r'^ {0,3}\[((?:\\[\s\S]|[^\[\]\\])+)\]:[ \t]*(<[^<>]*>|\S+)',
);
// A fenced block to its closing fence or the end, or a code span.
const _fenceEnd = r'[ \t]*(?=\n|(?![\s\S]))|(?![\s\S]))';
final _code = RegExp(
  r'^ {0,3}(`{3,})(?![^\n]*`)[\s\S]*?(?:\n {0,3}\1`*'
  '$_fenceEnd'
  r'|^ {0,3}(~{3,})[\s\S]*?(?:\n {0,3}\2~*'
  '$_fenceEnd'
  r'|(?<!`)(`+)(?!`)[\s\S]+?(?<!`)\3(?!`)',
  multiLine: true,
);
// A code span, an escaped character, or an entity.
final _literal = RegExp(
  r'(?<!`)(`+)(?!`)([\s\S]+?)(?<!`)\1(?!`)'
  r'|\\([!-/:-@\[-`{-~])'
  r'|&(#[0-9]{1,7}|#[xX][0-9a-fA-F]{1,6}|[A-Za-z][A-Za-z0-9]{1,31});',
);
final _emphasis = [
  RegExp(r'\*\*(?=\S)(.+?)(?<=\S)\*\*'),
  RegExp(r'(?<![A-Za-z0-9_])__(?=\S)(.+?)(?<=\S)__(?![A-Za-z0-9_])'),
  RegExp(r'\*(?=\S)(.+?)(?<=\S)\*'),
  RegExp(r'(?<![A-Za-z0-9_])_(?=\S)(.+?)(?<=\S)_(?![A-Za-z0-9_])'),
];
