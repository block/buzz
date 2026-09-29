import 'package:flutter/services.dart';

/// Message-level RTL support for Arabic-script text (Arabic, Persian/Farsi,
/// Urdu). Direction is decided per paragraph: each paragraph follows the
/// first strong directional character typed in it. When a paragraph has no
/// strong character of its own (empty, or only neutral characters), it
/// follows the first paragraph's direction.
final _rtlLeadingPattern = RegExp(
  '^[^A-Za-z\\u00C0-\\u024F]*'
  '[\\u0600-\\u06FF\\u0750-\\u077F'
  '\\u08A0-\\u08FF\\uFB50-\\uFDFF\\uFE70-\\uFEFF]',
);

/// True when [paragraph] should be laid out right-to-left.
bool isRtlParagraph(String? paragraph) {
  if (paragraph == null || paragraph.isEmpty) return false;
  return _rtlLeadingPattern.hasMatch(paragraph);
}

/// The direction for [content] as a whole: the first paragraph's first
/// strong character decides; if it is neutral, the first paragraph with a
/// strong RTL character decides.
TextDirection textDirectionFor(String? content) {
  if (content == null || content.trim().isEmpty) return TextDirection.ltr;
  final paragraphs =
      content
          .split(RegExp(r'\n[ \t]*\n'))
          .where((p) => p.trim().isNotEmpty)
          .toList();
  if (paragraphs.isEmpty) return TextDirection.ltr;
  if (isRtlParagraph(paragraphs.first)) return TextDirection.rtl;
  // First paragraph has no leading RTL char — but a later paragraph may.
  for (final p in paragraphs.skip(1)) {
    if (isRtlParagraph(p)) return TextDirection.rtl;
  }
  return TextDirection.ltr;
}
