import 'dart:convert';
import 'dart:typed_data';

/// Return a cross-platform, relay-valid display name for an attachment.
String safeAttachmentFilename(String filename) {
  final segments = filename.split(RegExp(r'[/\\]'));
  final basename = segments.isEmpty ? '' : segments.last;
  final preserveCalendarExtension = hasCalendarExtension(basename);
  final source = preserveCalendarExtension
      ? basename.substring(0, basename.length - '.ics'.length)
      : basename;
  final byteLimit = preserveCalendarExtension ? 255 - '.ics'.length : 255;
  final sanitized = StringBuffer();
  var byteLength = 0;

  for (final rune in source.runes) {
    if ((rune >= 0 && rune <= 0x1f) || (rune >= 0x7f && rune <= 0x9f)) {
      continue;
    }

    final character = String.fromCharCode(rune);
    final characterByteLength = utf8.encode(character).length;
    if (byteLength + characterByteLength > byteLimit) break;

    sanitized.write(character);
    byteLength += characterByteLength;
  }

  final safeBasename = sanitized.toString().trim();
  if (preserveCalendarExtension) {
    return '${safeBasename.isEmpty ? 'calendar' : safeBasename}.ics';
  }
  return safeBasename.isEmpty ? 'file' : safeBasename;
}

/// Return whether [filename] carries the allowlisted calendar extension.
bool hasCalendarExtension(String filename) {
  return filename.toLowerCase().endsWith('.ics');
}

/// Validate the bounded UTF-8 VCALENDAR envelope accepted by the relay.
void validateCalendarBytes(Uint8List bytes) {
  late final String text;
  try {
    text = utf8.decode(bytes);
  } on FormatException {
    throw Exception('invalid calendar file: expected UTF-8 text');
  }
  if (bytes.contains(0)) {
    throw Exception('invalid calendar file: NUL bytes are not allowed');
  }
  final lines = text
      .split(RegExp(r'\r?\n'))
      .map((line) => line.trim())
      .where((line) => line.isNotEmpty)
      .toList(growable: false);
  if (lines.isEmpty ||
      lines.first.toUpperCase() != 'BEGIN:VCALENDAR' ||
      lines.last.toUpperCase() != 'END:VCALENDAR') {
    throw Exception('invalid calendar file: missing VCALENDAR envelope');
  }
}
