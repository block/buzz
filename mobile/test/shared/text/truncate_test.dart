import 'package:buzz/shared/text/truncate.dart';
import 'package:flutter_test/flutter_test.dart';

void main() {
  test('leaves text that is already short enough', () {
    expect(truncateToCharacters('hello', 100), 'hello');
    expect(truncateToCharacters('hello', 5), 'hello');
  });

  test('never ends on half a surrogate pair', () {
    final body = '${'x' * 199}\u{1F389} more text';
    final cut = truncateToCharacters(body, 200);
    expect(cut, '${'x' * 199}\u{1F389}');
    // A lone surrogate would leave the last code unit in D800..DBFF.
    expect(cut.codeUnitAt(cut.length - 1) & 0xFC00 == 0xD800, isFalse);
  });

  test('counts characters, not code units', () {
    const party = '\u{1F389}';
    expect(truncateToCharacters(party * 3, 2), party * 2);
    expect(truncateToCharacters(party * 3, 2).runes.length, 2);
  });

  test('handles the degenerate limits', () {
    expect(truncateToCharacters('hello', 0), '');
    expect(truncateToCharacters('', 10), '');
  });

  test('appends the ellipsis only when it actually cut', () {
    expect(truncateWithEllipsis('hello', 100, '...'), 'hello');
    expect(truncateWithEllipsis('hello', 5, '...'), 'hello');
    expect(truncateWithEllipsis('hello there', 5, '...'), 'hello...');
  });

  test('does not cut the ellipsis into an emoji', () {
    final body = '${'x' * 199}\u{1F389} more text';
    expect(truncateWithEllipsis(body, 200, '...'), '${'x' * 199}\u{1F389}...');
  });
}
