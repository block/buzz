import 'package:buzz/shared/hosted/account_identity_store.dart';
import 'package:buzz/shared/identity_transfer/nsec_import_sheet.dart';
import 'package:buzz/shared/utils/string_utils.dart';
import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:nostr/nostr.dart' as nostr;

void main() {
  final keys = nostr.Keys('1'.padLeft(64, '0'));

  testWidgets(
    'paste requires public preview then confirmation before secure save',
    (tester) async {
      String? imported;
      await tester.pumpWidget(
        MaterialApp(
          home: Scaffold(
            body: NsecImportSheet(
              onImport: (nsec) async {
                imported = nsec;
              },
            ),
          ),
        ),
      );
      await tester.enterText(find.byType(TextField), keys.nsec);
      await tester.tap(find.text('Preview identity'));
      await tester.pumpAndSettle();
      expect(find.text(fullNpub(keys.public)!), findsOneWidget);
      expect(imported, isNull);
      await tester.tap(find.text('Import this identity'));
      await tester.pumpAndSettle();
      expect(imported, keys.nsec);
      await tester.pump();
    },
  );

  testWidgets('file import previews and occupied identity stays unchanged', (
    tester,
  ) async {
    var attempts = 0;
    await tester.pumpWidget(
      MaterialApp(
        home: Scaffold(
          body: NsecImportSheet(
            pickFile: () async => keys.nsec,
            onImport: (nsec) async {
              attempts++;
              throw AccountIdentityFailure.occupied;
            },
          ),
        ),
      ),
    );
    await tester.tap(find.text('Choose nsec file'));
    await tester.pumpAndSettle();
    expect(find.text(fullNpub(keys.public)!), findsOneWidget);
    await tester.tap(find.text('Import this identity'));
    await tester.pumpAndSettle();
    expect(
      find.textContaining('already has a different identity'),
      findsOneWidget,
    );
    expect(attempts, 1);
    await tester.enterText(find.byType(TextField), 'not a key');
    await tester.pump();
    expect(find.text('Import this identity'), findsNothing);
    await tester.tap(find.text('Preview identity'));
    await tester.pump();
    expect(find.text('Enter a valid nsec private key.'), findsOneWidget);
  });
}
