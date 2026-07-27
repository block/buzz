import 'package:buzz/shared/theme/theme.dart';
import 'package:buzz/shared/widgets/message_author_meta.dart';
import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';

void main() {
  testWidgets('constrains long metadata at large accessible text sizes', (
    tester,
  ) async {
    const timestampKey = Key('author-timestamp');

    await tester.pumpWidget(
      MaterialApp(
        theme: AppTheme.light(),
        home: const MediaQuery(
          data: MediaQueryData(textScaler: TextScaler.linear(2)),
          child: Scaffold(
            body: SizedBox(
              width: 220,
              child: MessageAuthorMeta(
                displayName: 'A very long display name',
                username: 'a-very-long-username',
                timestamp: 'Mar 15, 2025',
                timestampKey: timestampKey,
                nameColor: Colors.black,
                metadataColor: Colors.grey,
              ),
            ),
          ),
        ),
      ),
    );
    await tester.pumpAndSettle();

    final timestamp = tester.widget<Text>(find.byKey(timestampKey));
    expect(timestamp.maxLines, 1);
    expect(timestamp.overflow, TextOverflow.ellipsis);
    expect(tester.takeException(), isNull);
  });
}
