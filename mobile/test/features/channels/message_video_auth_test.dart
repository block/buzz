import 'dart:isolate';

import 'package:buzz/features/channels/message_content.dart';
import 'package:buzz/shared/relay/media_auth.dart';
import 'package:buzz/shared/theme/theme.dart';
import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';

class _FailingMediaAuth extends MediaGetAuthService {
  _FailingMediaAuth(this.error)
    : super(baseUrl: 'https://example.com', nsec: null);

  final Object error;
  final requests = <String>[];

  @override
  Future<Map<String, String>> headersForAsync(String url) async {
    requests.add(url);
    throw error;
  }
}

void main() {
  for (final failure in <String, Object>{
    'queue overflow': StateError('Media authentication queue is full'),
    'isolate startup': IsolateSpawnException('worker unavailable'),
  }.entries) {
    testWidgets('posterless video handles auth ${failure.key}', (tester) async {
      const url = 'https://example.com/media/clip.mp4';
      final auth = _FailingMediaAuth(failure.value);
      await tester.pumpWidget(
        ProviderScope(
          overrides: [mediaGetAuthServiceProvider.overrideWithValue(auth)],
          child: MaterialApp(
            theme: AppTheme.light(),
            home: const AppMarkdownTheme(
              child: Scaffold(
                body: MessageContent(
                  content: '![video]($url)',
                  tags: [
                    ['imeta', 'url $url', 'm video/mp4'],
                  ],
                ),
              ),
            ),
          ),
        ),
      );
      await tester.pumpAndSettle();
      expect(
        auth.requests,
        failure.value is IsolateSpawnException ? [url, url] : [url],
      );
      expect(find.text('Video attachment'), findsOneWidget);
      expect(tester.takeException(), isNull);
      await tester.pumpWidget(const SizedBox.shrink());
      await tester.pump();
      expect(tester.takeException(), isNull);
    });
  }
}
