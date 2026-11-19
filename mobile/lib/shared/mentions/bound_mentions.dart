import 'dart:math' as math;

import 'package:flutter/services.dart';

import 'mention_bindings.dart';

/// Where a picked mention sits in a draft: [start] is its `@`, [end] is
/// exclusive. The text between is the literal `@Label` the composer shows
/// as one chip. The binding to a recipient lives beside the range, so two
/// namesakes can both read `@Sam` and still notify two different people.
typedef MentionRange = ({int start, int end});

final _nameContinues = RegExp('^$mentionNameContinuation', unicode: true);

/// Whether a mention ending at [end] in [text] still ends at a name
/// boundary. A letter, mark or digit typed directly against it continues
/// the name (`@Bobby`), so the mention returns to prose and its recipient
/// goes; punctuation, quotes, hyphens, slashes and emoji keep both (`@Bob's`,
/// `@Bob’s`, `@Bob-bot`, `@Bob/Alice`). This is buzz-app's `mentionBoundary`
/// (`src/features/messages/mention-draft.ts`) with one difference its rich
/// editor does not need: this draft is Markdown source, so underscores that
/// close emphasis end the name (`_@Bob_`) and only underscores leading to a
/// name character continue it (`@Bob_smith`).
bool mentionEndsAtBoundary(String text, int end) =>
    end >= text.length || !_nameContinues.hasMatch(text.substring(end));

/// The range of [oldText] an edit replaced to produce [newText].
///
/// Identical neighbouring text makes a plain diff ambiguous: deleting the
/// first of `@Sam @Sam` reads the same as deleting the second, yet the two
/// chips notify different people. The selections tell them apart: a
/// selected range the edit replaced, a backspace ending at the caret or a
/// forward delete starting there (the new caret says which), or an
/// insertion at the caret. Any other edit, such as an IME rewriting the word
/// behind the caret, falls back to the longest common prefix and suffix.
TextRange editReplacedRange(
  String oldText,
  String newText, {
  required TextSelection oldSelection,
  required TextSelection newSelection,
}) {
  bool replaces(int start, int end) =>
      start >= 0 &&
      start <= end &&
      end <= oldText.length &&
      newText.length >= oldText.length - (end - start) &&
      newText.startsWith(oldText.substring(0, start)) &&
      newText.endsWith(oldText.substring(end));

  if (oldSelection.isValid && oldSelection.end <= oldText.length) {
    final start = oldSelection.start;
    final end = oldSelection.end;
    if (!oldSelection.isCollapsed) {
      if (replaces(start, end)) return TextRange(start: start, end: end);
    } else {
      final removed = oldText.length - newText.length;
      if (removed > 0) {
        final caret = newSelection.isValid ? newSelection.baseOffset : -1;
        final backspace = (start: start - removed, end: start);
        final forward = (start: start, end: start + removed);
        for (final candidate
            in caret == start ? [forward, backspace] : [backspace, forward]) {
          if (replaces(candidate.start, candidate.end)) {
            return TextRange(start: candidate.start, end: candidate.end);
          }
        }
      } else if (replaces(start, start)) {
        return TextRange(start: start, end: start);
      }
    }
  }
  final limit = math.min(oldText.length, newText.length);
  var prefix = 0;
  while (prefix < limit &&
      oldText.codeUnitAt(prefix) == newText.codeUnitAt(prefix)) {
    prefix++;
  }
  var suffix = 0;
  while (suffix < limit - prefix &&
      oldText.codeUnitAt(oldText.length - 1 - suffix) ==
          newText.codeUnitAt(newText.length - 1 - suffix)) {
    suffix++;
  }
  return TextRange(start: prefix, end: oldText.length - suffix);
}

/// Carries [ranges], in text order, across an edit of the old text that
/// replaced [replaced] to produce [newText]. The result holds one entry per
/// input: its new range, or null when the edit reached into the mention or
/// left it no longer ending at a name boundary. Text typed before a mention
/// moves it; text typed at its end leaves it where it is.
///
/// An edit that knows it kept the replaced text, such as formatting wrapped
/// around a selection, passes [keptShift]: how far that text moved inside
/// the replacement (the length of the opening delimiter). Mentions wholly
/// inside [replaced] then move with it rather than being dropped.
List<MentionRange?> remapMentionRanges(
  List<MentionRange> ranges, {
  required TextRange replaced,
  required int oldLength,
  required String newText,
  int? keptShift,
}) {
  final delta = newText.length - oldLength;
  return [
    for (final range in ranges)
      () {
        final MentionRange moved;
        if (range.end <= replaced.start) {
          moved = range;
        } else if (range.start >= replaced.end) {
          moved = (start: range.start + delta, end: range.end + delta);
        } else if (keptShift != null &&
            range.start >= replaced.start &&
            range.end <= replaced.end) {
          moved = (start: range.start + keptShift, end: range.end + keptShift);
        } else {
          return null;
        }
        if (moved.start < 0 ||
            moved.end > newText.length ||
            moved.start >= moved.end ||
            newText.codeUnitAt(moved.start) != 0x40 /* @ */ ||
            !mentionEndsAtBoundary(newText, moved.end)) {
          return null;
        }
        return moved;
      }(),
  ];
}
