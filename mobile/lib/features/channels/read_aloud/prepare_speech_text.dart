/// Prepare message text for on-device speech.
///
/// Ports `preprocess_for_tts` from
/// `desktop/src-tauri/src/huddle/preprocessing.rs` and the attachment-line
/// strip from `textWithoutAttachments` in
/// `desktop/src/features/huddle/lib/ttsLiveMessages.ts`.
String prepareSpeechText(String content, {List<List<String>> tags = const []}) {
  final stripped = _collapseWhitespace(
    _expandNumbers(
      _stripEmoji(
        _stripMarkdownMarkers(
          _stripUrls(
            _stripInlineCode(
              _stripFencedCodeBlocks(_stripAttachmentLines(content, tags)),
            ),
          ),
        ),
      ),
    ),
  );
  // ".", ",", and other one-character leftovers would be spoken as
  // "period". Match the desktop filter.
  if (stripped.length <= 1) return '';
  return stripped;
}

/// Split [text] into utterances no longer than [maxLength] UTF-16 code units.
///
/// Android's `TextToSpeech.getMaxSpeechInputLength` rejects longer input.
/// Sentence boundaries win, then spaces, then a hard cut. A non-positive
/// limit means the platform has no limit.
List<String> splitSpeechChunks(String text, {required int maxLength}) {
  final trimmed = text.trim();
  if (trimmed.isEmpty) return const [];
  if (maxLength <= 0 || trimmed.length <= maxLength) return [trimmed];

  final chunks = <String>[];
  var rest = trimmed;
  while (rest.isNotEmpty) {
    if (rest.length <= maxLength) {
      chunks.add(rest);
      break;
    }
    final cut = _chunkCut(rest, maxLength);
    final chunk = rest.substring(0, cut).trim();
    if (chunk.isNotEmpty) chunks.add(chunk);
    rest = rest.substring(cut).trimLeft();
    if (cut == 0) break;
  }
  return chunks;
}

int _chunkCut(String text, int maxLength) {
  final window = text.substring(0, maxLength);
  final sentence = RegExp(r'[.!?]\s+').allMatches(window).toList();
  if (sentence.isNotEmpty) return sentence.last.end;
  final space = window.lastIndexOf(' ');
  if (space > 0) return space + 1;
  return maxLength;
}

String _stripAttachmentLines(String content, List<List<String>> tags) {
  final urls = <String>{
    for (final tag in tags)
      if (tag.isNotEmpty && tag.first == 'imeta')
        for (final field in tag.skip(1))
          if (field.startsWith('url ')) field.substring(4),
  };
  if (urls.isEmpty) return content;
  final withoutMedia = content
      .split('\n')
      .where((line) => !urls.any((url) => line.contains(']($url)')))
      .join('\n');
  return withoutMedia.replaceAllMapped(
    RegExp(r'(^|\n)\s*\|\|\s*\n(?:\s*\n)*\s*\|\|\s*(?=\n|$)', unicode: true),
    (match) => match.group(1)!,
  );
}

String _stripFencedCodeBlocks(String text) {
  return _replaceFenced(_replaceFenced(text, '```'), '~~~');
}

String _replaceFenced(String text, String fence) {
  final out = StringBuffer();
  var rest = text;
  while (true) {
    final start = rest.indexOf(fence);
    if (start < 0) {
      out.write(rest);
      break;
    }
    out.write(rest.substring(0, start));
    rest = rest.substring(start + fence.length);
    final newline = rest.indexOf('\n');
    if (newline >= 0) rest = rest.substring(newline + 1);
    final end = rest.indexOf(fence);
    out.write(' code block omitted ');
    if (end < 0) break;
    rest = rest.substring(end + fence.length);
    if (rest.startsWith('\n')) rest = rest.substring(1);
  }
  return out.toString();
}

String _stripInlineCode(String text) {
  final out = StringBuffer();
  var rest = text;
  while (true) {
    final start = rest.indexOf('`');
    if (start < 0) {
      out.write(rest);
      break;
    }
    out.write(rest.substring(0, start));
    rest = rest.substring(start + 1);
    final end = rest.indexOf('`');
    if (end < 0) {
      out.write(rest);
      break;
    }
    out.write(rest.substring(0, end));
    rest = rest.substring(end + 1);
  }
  return out.toString();
}

String _stripUrls(String text) {
  final out = StringBuffer();
  var rest = text;
  while (true) {
    final http = rest.indexOf('http://');
    final https = rest.indexOf('https://');
    final start = switch ((http, https)) {
      (-1, -1) => -1,
      (_, -1) => http,
      (-1, _) => https,
      _ => http < https ? http : https,
    };
    if (start < 0) {
      out.write(rest);
      break;
    }
    out.write(rest.substring(0, start));
    rest = rest.substring(start);
    final end = rest.indexOf(RegExp(r'''[\s)\]"']'''));
    final urlEnd = end < 0 ? rest.length : end;
    final token = rest.substring(0, urlEnd);
    rest = rest.substring(urlEnd);
    final last = token.isEmpty ? '' : token[token.length - 1];
    final keepPunct =
        (last == '.' || last == '!' || last == '?') &&
        (rest.isEmpty || RegExp(r'^\s').hasMatch(rest));
    out.write('link omitted');
    if (keepPunct) out.write(last);
  }
  return out.toString();
}

String _stripMarkdownMarkers(String text) {
  return _stripUnderscoreEmphasis(
    text
        .replaceAll('**', '')
        .replaceAll('__', '')
        .replaceAll('~~', '')
        .replaceAll('*', ''),
  );
}

String _stripUnderscoreEmphasis(String text) {
  final chars = text.runes.toList();
  final out = StringBuffer();
  var i = 0;
  while (i < chars.length) {
    if (chars[i] == 0x5F) {
      final prevBoundary = i == 0 || _isWhitespace(chars[i - 1]);
      final nextNonSpace = i + 1 < chars.length && !_isWhitespace(chars[i + 1]);
      if (prevBoundary && nextNonSpace) {
        var close = -1;
        for (var j = i + 1; j < chars.length; j++) {
          if (chars[j] != 0x5F) continue;
          if (_isWhitespace(chars[j - 1])) continue;
          final atEnd = j + 1 >= chars.length;
          if (atEnd ||
              _isWhitespace(chars[j + 1]) ||
              _isAsciiPunctuation(chars[j + 1])) {
            close = j;
            break;
          }
        }
        if (close >= 0) {
          out.write(String.fromCharCodes(chars.sublist(i + 1, close)));
          i = close + 1;
          continue;
        }
      }
    }
    out.writeCharCode(chars[i]);
    i++;
  }
  return out.toString();
}

String _stripEmoji(String text) {
  return String.fromCharCodes(text.runes.where((rune) => !_isEmoji(rune)));
}

bool _isEmoji(int rune) {
  return (rune >= 0x1F300 && rune <= 0x1FAFF) ||
      (rune >= 0x2600 && rune <= 0x27BF) ||
      (rune >= 0xFE00 && rune <= 0xFE0F) ||
      (rune >= 0x1F000 && rune <= 0x1F02F) ||
      (rune >= 0x1F0A0 && rune <= 0x1F0FF) ||
      rune == 0x200D ||
      rune == 0x20E3;
}

String _expandNumbers(String text) {
  final units = text.codeUnits;
  final out = StringBuffer();
  var i = 0;
  while (i < units.length) {
    final unit = units[i];
    if (unit >= 0x30 && unit <= 0x39) {
      final start = i;
      i++;
      while (i < units.length) {
        final next = units[i];
        if ((next >= 0x30 && next <= 0x39) || next == 0x3A || next == 0x2E) {
          i++;
        } else {
          break;
        }
      }
      out.write(_expandNumericToken(text.substring(start, i)));
    } else {
      out.writeCharCode(unit);
      i++;
    }
  }
  return out.toString();
}

String _expandNumericToken(String token) {
  var end = token.length;
  while (end > 0) {
    final unit = token.codeUnitAt(end - 1);
    if (unit >= 0x30 && unit <= 0x39) break;
    end--;
  }
  token = token.substring(0, end);
  final colon = token.indexOf(':');
  if (colon >= 0) {
    final hour = int.tryParse(token.substring(0, colon));
    final minute = int.tryParse(token.substring(colon + 1));
    if (hour != null && minute != null && hour < 24 && minute < 60) {
      if (minute == 0) return _intToWords(hour);
      final minuteWords = minute < 10
          ? ' oh ${_intToWords(minute)}'
          : ' ${_intToWords(minute)}';
      return '${_intToWords(hour)}$minuteWords';
    }
    return token;
  }
  if (token.codeUnits.every((unit) => unit >= 0x30 && unit <= 0x39)) {
    final value = int.tryParse(token);
    if (value != null && value <= 999999) return _intToWords(value);
  }
  return token;
}

const _ones = [
  'zero',
  'one',
  'two',
  'three',
  'four',
  'five',
  'six',
  'seven',
  'eight',
  'nine',
  'ten',
  'eleven',
  'twelve',
  'thirteen',
  'fourteen',
  'fifteen',
  'sixteen',
  'seventeen',
  'eighteen',
  'nineteen',
];

const _tens = [
  '',
  '',
  'twenty',
  'thirty',
  'forty',
  'fifty',
  'sixty',
  'seventy',
  'eighty',
  'ninety',
];

String _intToWords(int value) {
  if (value < 20) return _ones[value];
  if (value < 100) {
    final ten = _tens[value ~/ 10];
    final one = value % 10;
    return one == 0 ? ten : '$ten ${_ones[one]}';
  }
  if (value < 1000) {
    final hundreds = '${_ones[value ~/ 100]} hundred';
    final remainder = value % 100;
    return remainder == 0 ? hundreds : '$hundreds ${_intToWords(remainder)}';
  }
  final thousands = '${_intToWords(value ~/ 1000)} thousand';
  final remainder = value % 1000;
  return remainder == 0 ? thousands : '$thousands ${_intToWords(remainder)}';
}

String _collapseWhitespace(String text) {
  final out = StringBuffer();
  var previousSpace = true;
  for (final rune in text.runes) {
    if (_isWhitespace(rune)) {
      if (!previousSpace) {
        out.write(' ');
        previousSpace = true;
      }
    } else {
      out.writeCharCode(rune);
      previousSpace = false;
    }
  }
  final collapsed = out.toString();
  return collapsed.endsWith(' ')
      ? collapsed.substring(0, collapsed.length - 1)
      : collapsed;
}

bool _isWhitespace(int rune) {
  return RegExp(r'^\s$', unicode: true).hasMatch(String.fromCharCode(rune));
}

bool _isAsciiPunctuation(int rune) {
  return (rune >= 0x21 && rune <= 0x2F) ||
      (rune >= 0x3A && rune <= 0x40) ||
      (rune >= 0x5B && rune <= 0x60) ||
      (rune >= 0x7B && rune <= 0x7E);
}
