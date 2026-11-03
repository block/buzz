import 'package:buzz/shared/hosted/account_identity_store.dart';
import 'package:buzz/shared/identity_transfer/nsec_import_sheet.dart';
import 'package:buzz/shared/utils/string_utils.dart';
import 'package:flutter/cupertino.dart';
import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:nostr/nostr.dart' as nostr;

void main() {
  final keys = nostr.Keys('1'.padLeft(64, '0'));

  for (final platform in [TargetPlatform.android, TargetPlatform.iOS]) {
    testWidgets(
      '$platform paste requires public preview then confirmation before secure save',
      (tester) async {
        String? imported;
        await tester.pumpWidget(
          MaterialApp(
            home: Scaffold(
              body: Builder(
                builder: (context) => TextButton(
                  onPressed: () => showNsecImportSheet(
                    context,
                    onImport: (nsec) async {
                      imported = nsec;
                    },
                  ),
                  child: const Text('Open import'),
                ),
              ),
            ),
          ),
        );
        await tester.tap(find.text('Open import'));
        await tester.pumpAndSettle();
        final input = platform == TargetPlatform.iOS
            ? find.byType(CupertinoTextField)
            : find.byType(TextField);
        if (platform == TargetPlatform.iOS) {
          final field = tester.widget<CupertinoTextField>(input);
          expect(field.obscureText, isTrue);
          expect(field.autocorrect, isFalse);
          expect(field.enableIMEPersonalizedLearning, isFalse);
        }
        await tester.enterText(input, keys.nsec);
        await tester.tap(find.text('Preview identity'));
        await tester.pumpAndSettle();
        expect(find.text(fullNpub(keys.public)!), findsOneWidget);
        expect(imported, isNull);
        await tester.ensureVisible(find.text('Import this identity'));
        await tester.tap(find.text('Import this identity'));
        await tester.pumpAndSettle();
        expect(imported, keys.nsec);
        expect(find.text('Open import'), findsOneWidget);
        expect(find.text('Private key (nsec)'), findsNothing);
      },
      variant: TargetPlatformVariant({platform}),
    );
  }

  testWidgets('iOS import waits for its containing sheet before focusing', (
    tester,
  ) async {
    Future<void> show(bool autofocus) => tester.pumpWidget(
      MaterialApp(
        home: Scaffold(
          body: NsecImportSheet(autofocus: autofocus, onImport: (_) async {}),
        ),
      ),
    );
    await show(false);
    await tester.pump();
    expect(
      tester.widget<EditableText>(find.byType(EditableText)).focusNode.hasFocus,
      isFalse,
    );
    await show(true);
    await tester.pump();
    expect(
      tester.widget<EditableText>(find.byType(EditableText)).focusNode.hasFocus,
      isTrue,
    );
  }, variant: TargetPlatformVariant({TargetPlatform.iOS}));

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
