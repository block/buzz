import 'dart:io';

import 'package:flutter_test/flutter_test.dart';

void main() {
  test('QR scanning stays on the Google-independent dependency path', () {
    final pubspec = File('pubspec.yaml').readAsStringSync();
    final lockfile = File('pubspec.lock').readAsStringSync();
    final podLock = File('ios/Podfile.lock').readAsStringSync();
    final androidBuild = File(
      'android/app/build.gradle.kts',
    ).readAsStringSync();

    expect(
      pubspec,
      contains(RegExp(r'^  flutter_zxing: \^2\.3\.0$', multiLine: true)),
    );
    expect(
      lockfile,
      contains(
        RegExp(
          r'^  flutter_zxing:\n    dependency: "direct main"$',
          multiLine: true,
        ),
      ),
    );
    expect(podLock, contains('flutter_zxing'));
    expect(
      androidBuild,
      contains(
        'variant.runtimeConfiguration.incoming.resolutionResult.allComponents',
      ),
    );
    expect(
      androidBuild,
      isNot(contains('getByName("debugRuntimeClasspath").resolve()')),
    );

    final packageMetadata = '$pubspec\n$lockfile\n$podLock'.toLowerCase();
    for (final forbidden in const [
      'mobile_scanner',
      'google_mlkit',
      'firebase_ml',
      'com.google.mlkit',
      'com.google.firebase:firebase-ml',
      'play-services-mlkit',
    ]) {
      expect(
        packageMetadata,
        isNot(contains(forbidden)),
        reason: 'forbidden QR dependency marker: $forbidden',
      );
    }

    const legacyImport = 'package:mobile_scanner/';
    final sourceFiles = Directory('lib')
        .listSync(recursive: true)
        .whereType<File>()
        .where((file) => file.path.endsWith('.dart'));
    for (final file in sourceFiles) {
      expect(
        file.readAsStringSync(),
        isNot(contains(legacyImport)),
        reason: 'legacy scanner import in ${file.path}',
      );
    }
  });
}
