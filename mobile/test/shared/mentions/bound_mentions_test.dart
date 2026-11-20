import 'package:buzz/shared/mentions/bound_mentions.dart';
import 'package:flutter/services.dart';
import 'package:flutter_test/flutter_test.dart';

/// The rules that carry a picked mention across edits (buzz-app
/// `src/features/messages/mention-draft.ts`; project recommendation 3).
void main() {
  test('a mention ends at a name boundary unless a name character follows', () {
    for (final suffix in [
      "'s",
      '’s',
      '-bot',
      '/Alice',
      ', ok',
      '.',
      '!',
      '"',
      ')',
      '\u{1F600}',
      ' x',
      '',
      // Underscores that close Markdown emphasis, as the Italic button
      // writes them around a chip.
      '_',
      '_ x',
      '__',
      '_.',
    ]) {
      expect(mentionEndsAtBoundary('@Bob$suffix', 4), isTrue, reason: suffix);
    }
    for (final suffix in ['by', '2', '_x', '__x', '_2', 'é', '́']) {
      expect(mentionEndsAtBoundary('@Bob$suffix', 4), isFalse, reason: suffix);
    }
  });

  test(
    'the replaced range follows the selection when the text is ambiguous',
    () {
      TextRange edit(
        String old,
        String next, {
        required int from,
        int? to,
        int? caret,
      }) => editReplacedRange(
        old,
        next,
        oldSelection: TextSelection(baseOffset: from, extentOffset: to ?? from),
        newSelection: TextSelection.collapsed(offset: caret ?? next.length),
      );
      // Deleting the first of two identical chips over a selection.
      expect(
        edit('@Sam @Sam', '@Sam', from: 0, to: 5),
        const TextRange(start: 0, end: 5),
      );
      // A widened backspace at the end of the first chip, and a forward delete
      // of the second from the same caret, remove different chips.
      expect(
        edit('@Sam @Sam', ' @Sam', from: 4, caret: 0),
        const TextRange(start: 0, end: 4),
      );
      expect(
        edit('@Sam @Sam', '@Sam ', from: 5, caret: 5),
        const TextRange(start: 5, end: 9),
      );
      // Typing at the caret.
      expect(
        edit('@Bob', "@Bob's", from: 4),
        const TextRange(start: 4, end: 4),
      );
      // An IME rewriting the word behind the caret: common prefix and suffix.
      expect(
        edit("@Bob's", '@Bob’s', from: 6),
        const TextRange(start: 4, end: 5),
      );
      // A replacement that removes more than sits before the caret.
      expect(
        edit('hello @Sam and more', '@x', from: 10),
        const TextRange(start: 0, end: 19),
      );
    },
  );

  test('ranges move with the text and drop when an edit reaches in', () {
    const ranges = [(start: 0, end: 4), (start: 5, end: 9)];
    List<MentionRange?> remap(TextRange replaced, String next) =>
        remapMentionRanges(
          ranges,
          replaced: replaced,
          oldLength: 9,
          newText: next,
        );
    // Text typed before a chip moves it.
    expect(remap(const TextRange(start: 5, end: 5), '@Sam hi @Sam'), [
      (start: 0, end: 4),
      (start: 8, end: 12),
    ]);
    // A suffix typed against a chip keeps it; a name character drops it.
    expect(remap(const TextRange(start: 4, end: 4), "@Sam's @Sam"), [
      (start: 0, end: 4),
      (start: 7, end: 11),
    ]);
    expect(remap(const TextRange(start: 4, end: 4), '@Samy @Sam'), [
      null,
      (start: 6, end: 10),
    ]);
    // Deleting the first chip leaves the second bound.
    expect(remap(const TextRange(start: 0, end: 4), ' @Sam'), [
      null,
      (start: 1, end: 5),
    ]);
    // Replacing a chip's range revokes it, even with the same text.
    expect(remap(const TextRange(start: 5, end: 9), '@Sam @Sam'), [
      (start: 0, end: 4),
      null,
    ]);
  });
}
