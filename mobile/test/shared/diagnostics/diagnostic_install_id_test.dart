import 'dart:io';

import 'package:buzz/shared/diagnostics/diagnostic_startup.dart';
import 'package:flutter_test/flutter_test.dart';

void main() {
  // A damaged file is replaced, so reports still carry one install ID from
  // then on.
  for (final (name, contents) in <(String, List<int>)>[
    ('empty', []),
    ('blank', [0x20, 0x0a]),
    ('not text', [0xff, 0xfe, 0xc3, 0x28]),
  ]) {
    test('a $name install ID file is replaced with one that is kept', () async {
      final directory = await Directory.systemTemp.createTemp('install');
      addTearDown(() => directory.delete(recursive: true));
      await File(
        '${directory.path}/diagnostics_install_id',
      ).writeAsBytes(contents);

      final id = await diagnosticInstallId(directory: () async => directory);

      expect(id, isNotNull);
      expect(id, isNotEmpty);
      expect(await diagnosticInstallId(directory: () async => directory), id);
    });
  }
}
