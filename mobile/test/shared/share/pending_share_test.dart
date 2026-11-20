import 'dart:convert';
import 'dart:io';

import 'package:buzz/shared/share/pending_share.dart';
import 'package:flutter_test/flutter_test.dart';

void main() {
  late Directory shares;
  setUp(() => shares = Directory.systemTemp.createTempSync('pending-shares-'));
  tearDown(() => shares.deleteSync(recursive: true));

  Future<PendingShare> load(Map<String, Object?> manifest) async {
    const id = '77777777-7777-4777-8777-777777777777';
    final directory = Directory('${shares.path}/$id')..createSync();
    File(
      '${directory.path}/share.json',
    ).writeAsStringSync(jsonEncode({'version': 1, ...manifest}));
    return loadPendingShare(id, sharesDirectory: () async => shares);
  }

  test('documents keep the name they were shared under', () async {
    final share = await load({
      'text': [],
      'files': [],
      'documents': [
        {'file': 'A1.pdf', 'name': 'Quarterly report.pdf'},
        {'file': 'B2.webarchive', 'name': ' '},
      ],
    });
    expect(share.documents.map((d) => d.name), [
      'Quarterly report.pdf',
      'File',
    ]);
    expect(share.documents.first.path, endsWith('/A1.pdf'));
  });

  test('a manifest from before documents still loads', () async {
    final share = await load({
      'text': ['hi'],
      'files': [],
    });
    expect(share.documents, isEmpty);
  });

  test('a document path cannot leave the share directory', () async {
    expect(
      load({
        'text': [],
        'files': [],
        'documents': [
          {'file': '../escape.pdf', 'name': 'x.pdf'},
        ],
      }),
      throwsA(isA<PendingShareUnavailable>()),
    );
  });
}
