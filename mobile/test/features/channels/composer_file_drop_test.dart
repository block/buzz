import 'dart:io';

import 'package:buzz/features/channels/composer_file_drop.dart';
import 'package:flutter/services.dart';
import 'package:flutter_test/flutter_test.dart';

void main() {
  TestWidgetsFlutterBinding.ensureInitialized();

  const channel = MethodChannel('test/composer_drop');
  final enabledCalls = <Object?>[];

  setUp(() {
    enabledCalls.clear();
    TestDefaultBinaryMessengerBinding.instance.defaultBinaryMessenger
        .setMockMethodCallHandler(channel, (call) async {
          enabledCalls.add(call.arguments);
          return null;
        });
  });

  tearDown(() {
    TestDefaultBinaryMessengerBinding.instance.defaultBinaryMessenger
        .setMockMethodCallHandler(channel, null);
  });

  MethodCall dropCall(List<Map<String, Object?>> files, {int failures = 0}) =>
      MethodCall('didDrop', {'files': files, 'failures': failures});

  test('parses dropped files and fills missing names and types', () {
    final drop = ComposerFileDrop.parseDrop({
      'files': [
        {
          'path': '/tmp/a/report.pdf',
          'name': 'report.pdf',
          'mimeType': 'application/pdf',
        },
        {'path': '/tmp/b/blob'},
        {'path': ''},
        'junk',
      ],
      'failures': 2,
    });

    expect(drop.files.map((file) => file.name), ['report.pdf', 'blob']);
    expect(drop.files.last.mimeType, 'application/octet-stream');
    expect(drop.failures, 2);
    expect(ComposerFileDrop.parseDrop(null).files, isEmpty);
  });

  test('enables native drops only while a composer is registered', () async {
    final fileDrop = ComposerFileDrop(channel: channel);
    final unregisterFirst = fileDrop.register((_) {});
    final unregisterSecond = fileDrop.register((_) {});
    await pumpEventQueue();
    expect(enabledCalls, [true]);

    unregisterFirst();
    unregisterFirst();
    await pumpEventQueue();
    expect(enabledCalls, [true]);

    unregisterSecond();
    await pumpEventQueue();
    expect(enabledCalls, [true, false]);
  });

  test('routes a drop to the most recently registered composer', () async {
    final fileDrop = ComposerFileDrop(channel: channel);
    final first = <ComposerDrop>[];
    final second = <ComposerDrop>[];
    fileDrop.register(first.add);
    final unregisterSecond = fileDrop.register(second.add);

    await fileDrop.handleCall(
      dropCall([
        {'path': '/tmp/x.txt'},
      ]),
    );
    expect(first, isEmpty);
    expect(second.single.files.single.path, '/tmp/x.txt');

    unregisterSecond();
    await fileDrop.handleCall(
      dropCall([
        {'path': '/tmp/y.txt'},
      ]),
    );
    expect(first.single.files.single.path, '/tmp/y.txt');
  });

  test('deletes dropped copies when no composer is listening', () async {
    final directory = Directory.systemTemp.createTempSync('composer-drop');
    addTearDown(() => directory.deleteSync(recursive: true));
    final file = File('${directory.path}/orphan.txt')..writeAsStringSync('x');
    final fileDrop = ComposerFileDrop(channel: channel);

    await fileDrop.handleCall(
      dropCall([
        {'path': file.path},
      ]),
    );

    expect(file.existsSync(), isFalse);
  });
}
