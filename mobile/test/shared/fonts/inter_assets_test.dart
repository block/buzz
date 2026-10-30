import 'dart:io';

import 'package:crypto/crypto.dart';
import 'package:flutter_test/flutter_test.dart';

void main() {
  // These are the reproducible outputs of scripts/prepare-inter.py. Pin the
  // complete artifacts so CI rejects restored heart/warning mappings and any
  // unintended changes to other glyphs, metrics, or language coverage.
  // On a font update, first verify the transformation as documented in
  // assets/fonts/README.md, then update these expected digests.
  const expected = {
    'InterVariable.ttf':
        'c6540e8bbb50fac6ccdb0f0c01301bf195c95985c21fe9767cffe6963cc23afd',
    'InterVariable-Italic.ttf':
        '74a6ff09b186744377e1ff68772e50754f5113079e1bd1abf2e4ab1942c1d9b6',
  };

  for (final entry in expected.entries) {
    test('${entry.key} retains verified native emoji fallback coverage', () {
      final bytes = File('assets/fonts/${entry.key}').readAsBytesSync();
      expect(
        sha256.convert(bytes).toString(),
        entry.value,
        reason:
            'Bundled Inter must match the verified heart/warning fallback '
            'artifact. See assets/fonts/README.md before updating this digest.',
      );
    });
  }
}
