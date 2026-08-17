import 'dart:io';

import 'package:integration_test/integration_test_driver_extended.dart';

Future<void> main() async {
  final outputDirectory = Directory(
    Platform.environment['BUZZ_MOBILE_SCREENSHOT_DIR'] ??
        'test-results/mobile-emulator',
  );
  outputDirectory.createSync(recursive: true);

  await integrationDriver(
    onScreenshot: (name, bytes, [args]) async {
      File('${outputDirectory.path}/$name.png').writeAsBytesSync(bytes);
      return true;
    },
    writeResponseOnFailure: true,
  );
}
