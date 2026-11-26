import 'dart:convert';
import 'dart:io';
import 'dart:typed_data';

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

  // A read that fails, for instance with no file descriptor free, leaves the
  // saved ID for the next launch rather than replacing it.
  test('a file that fails to read keeps its install ID', () async {
    final directory = await Directory.systemTemp.createTemp('install');
    addTearDown(() => directory.delete(recursive: true));
    const saved = '6c0fb3a6-5b7e-4d58-9a40-1c2e2f0b7d11';
    final file = File('${directory.path}/diagnostics_install_id');
    await file.writeAsString(saved);

    final id = await IOOverrides.runZoned(
      () => diagnosticInstallId(directory: () async => directory),
      createFile: (_) => _UnreadableFile(file),
    );

    expect(id, isNull);
    expect(await file.readAsString(), saved);
  });
}

/// [file], except that every read fails as the operating system refused it,
/// while writes still succeed.
class _UnreadableFile implements File {
  _UnreadableFile(this.file);

  final File file;

  FileSystemException get _failure =>
      FileSystemException('Cannot open file', path, const OSError('', 24));

  @override
  String get path => file.path;

  @override
  Future<bool> exists() => file.exists();

  @override
  Future<Uint8List> readAsBytes() async => throw _failure;

  @override
  Future<String> readAsString({Encoding encoding = utf8}) async =>
      throw _failure;

  @override
  Future<File> writeAsString(
    String contents, {
    FileMode mode = FileMode.write,
    Encoding encoding = utf8,
    bool flush = false,
  }) => file.writeAsString(
    contents,
    mode: mode,
    encoding: encoding,
    flush: flush,
  );

  @override
  Future<File> writeAsBytes(
    List<int> bytes, {
    FileMode mode = FileMode.write,
    bool flush = false,
  }) => file.writeAsBytes(bytes, mode: mode, flush: flush);

  @override
  dynamic noSuchMethod(Invocation invocation) => super.noSuchMethod(invocation);
}
