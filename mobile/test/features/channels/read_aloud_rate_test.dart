import 'package:buzz/features/channels/read_aloud/read_aloud_engine.dart';
import 'package:flutter_test/flutter_test.dart';

void main() {
  test('1.3x maps to the flutter_tts rate both platforms treat as 1.3x', () {
    expect(flutterTtsRateForMultiplier(readAloudSpeed), 0.65);
  });

  test('prefers an offline Michael voice when the engine lists one', () {
    final voice = preferredReadAloudVoice([
      {'name': 'en-us-x-sfg-local', 'locale': 'en-US'},
      {'name': 'Michael', 'locale': 'en-US', 'network_required': '1'},
      {'name': 'michael-offline', 'locale': 'en-GB', 'network_required': '0'},
    ]);

    expect(voice?['name'], 'michael-offline');
  });

  test('leaves the system voice alone when Michael is not listed', () {
    expect(
      preferredReadAloudVoice([
        {'name': 'en-us-x-sfg-local', 'locale': 'en-US'},
      ]),
      isNull,
    );
  });
}
