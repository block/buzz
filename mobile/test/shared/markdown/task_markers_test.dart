import 'package:flutter_test/flutter_test.dart';

import 'package:buzz/shared/markdown/task_markers.dart';

/// Mirrors `desktop/src/shared/lib/toggleTaskMarker.test.mjs`. The two
/// implementations must agree case for case: a box checked on the phone has to
/// land on the same line the desktop would have rewritten.
void main() {
  test('checks the addressed task and leaves its siblings alone', () {
    const source = '- [ ] uno\n- [ ] dos\n- [ ] tres';
    expect(
      toggleTaskMarker(source, 1, true),
      '- [ ] uno\n- [x] dos\n- [ ] tres',
    );
  });

  test('unchecks an already-checked task', () {
    expect(toggleTaskMarker('- [x] hecho', 0, false), '- [ ] hecho');
  });

  test('preserves indentation, bullet style and trailing text', () {
    const source = '  * [ ]   revisar   \n1. [ ] segundo';
    expect(
      toggleTaskMarker(source, 0, true),
      '  * [x]   revisar   \n1. [ ] segundo',
    );
    expect(
      toggleTaskMarker(source, 1, true),
      '  * [ ]   revisar   \n1. [x] segundo',
    );
  });

  test('skips markers inside fenced code, which render as code', () {
    const source = '```\n- [ ] ejemplo\n```\n- [ ] real';
    expect(countTaskMarkers(source), 1);
    expect(
      toggleTaskMarker(source, 0, true),
      '```\n- [ ] ejemplo\n```\n- [x] real',
    );
  });

  test('handles tilde fences', () {
    const source = '~~~\n- [ ] dentro\n~~~\n- [ ] fuera';
    expect(countTaskMarkers(source), 1);
  });

  test('a bracket with no space after it is not a task marker', () {
    expect(countTaskMarkers('- [ ]sin-espacio'), 0);
    expect(toggleTaskMarker('- [ ]sin-espacio', 0, true), isNull);
  });

  test('returns null when the ordinal resolves to nothing', () {
    expect(toggleTaskMarker('- [ ] solo una', 3, true), isNull);
    expect(toggleTaskMarker('sin tareas', 0, true), isNull);
  });

  test('rejects a negative ordinal instead of writing something arbitrary', () {
    expect(toggleTaskMarker('- [ ] uno', -1, true), isNull);
  });

  test('counts across mixed prose and lists', () {
    const source = 'texto\n\n- [ ] a\n- [x] b\n\notro\n\n1) [ ] c';
    expect(countTaskMarkers(source), 3);
    expect(
      toggleTaskMarker(source, 2, true),
      'texto\n\n- [ ] a\n- [x] b\n\notro\n\n1) [x] c',
    );
  });

  test('duplicate task text still resolves by position, not by content', () {
    // The ordinal is the only identity a tapped checkbox has, so two tasks
    // worded identically must remain independently checkable.
    const source = '- [ ] revisar\n- [ ] revisar';
    expect(toggleTaskMarker(source, 1, true), '- [ ] revisar\n- [x] revisar');
    expect(toggleTaskMarker(source, 0, true), '- [x] revisar\n- [ ] revisar');
  });
}
