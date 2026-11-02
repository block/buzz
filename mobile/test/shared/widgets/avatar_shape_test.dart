import 'dart:io';

import 'package:buzz/shared/widgets/avatar_shape.dart';
import 'package:flutter/painting.dart';
import 'package:flutter_test/flutter_test.dart';

void main() {
  test('agent squircle follows the desktop mask geometry', () {
    // Desktop's avatar-squircle.svg passes through (.089, .089) at each
    // corner's midpoint, outside a circle and inside the square.
    const rect = Rect.fromLTWH(0, 0, 100, 100);
    final path = avatarPath(rect, isAgent: true);
    expect(path.getBounds(), rect);
    for (final (point, inside) in [
      (const Offset(9.5, 9.5), true),
      (const Offset(90.5, 9.5), true),
      (const Offset(90.5, 90.5), true),
      (const Offset(9.5, 90.5), true),
      (const Offset(8, 8), false),
      (const Offset(92, 8), false),
      (const Offset(92, 92), false),
      (const Offset(8, 92), false),
      // Inside the old 30% rounded rect, outside the squircle.
      (const Offset(1.9, 22), false),
      (const Offset(98.1, 78), false),
    ]) {
      expect(path.contains(point), inside, reason: '$point');
    }
    // People keep circles: the squircle corner points fall outside them.
    expect(
      avatarPath(rect, isAgent: false).contains(const Offset(9.5, 9.5)),
      isFalse,
    );
  });

  test('avatar shapes come only from avatarShape', () {
    // Agent avatars drifted into circles and 30% rounded rects because each
    // surface picked its own shape. Shape decisions keyed on agent identity,
    // the old agent radius idiom, and person shapes on agent-only surfaces
    // must use avatar_shape.dart. Best effort: it matches the patterns that
    // produced past drift, not every possible spelling.
    final inlineAgentShape = RegExp(
      r'\b(isAgent|isBot|agent)\b\s*\?\s*(const\s+)?'
      r'(BoxShape|BorderRadius|Radius|CircleBorder|RoundedRectangleBorder|'
      r'RoundedSuperellipseBorder|ClipOval|ClipRRect|CircleAvatar)\b',
    );
    final inlineAgentClip = RegExp(
      r'if\s*\(\s*!?\s*(isAgent|isBot|agent)\s*\)\s*return\s+'
      r'(ClipOval|ClipRRect|CircleAvatar)\b',
    );
    // The old hand-rolled agent radius: 30% of a size, or 60% of a radius.
    final agentRadius = RegExp(
      r'Radius\.circular\(\s*[\w.()+\s]*\*\s*0\.[36]\s*,?\s*\)',
    );
    final personShape = RegExp(r'\b(ClipOval|CircleAvatar|BoxShape\.circle)\b');
    final violations = <String>[];
    for (final file in Directory('lib').listSync(recursive: true)) {
      if (file is! File || !file.path.endsWith('.dart')) continue;
      final path = file.path.replaceAll(r'\', '/');
      if (path.endsWith('shared/widgets/avatar_shape.dart')) continue;
      final source = file.readAsStringSync();
      for (final match in [
        ...inlineAgentShape.allMatches(source),
        ...inlineAgentClip.allMatches(source),
        ...agentRadius.allMatches(source),
        if (path.startsWith('lib/features/agents/'))
          ...personShape.allMatches(source),
      ]) {
        final line = '\n'.allMatches(source.substring(0, match.start)).length;
        violations.add('$path:${line + 1}: ${match[0]}');
      }
    }
    expect(violations, isEmpty);
  });
}
