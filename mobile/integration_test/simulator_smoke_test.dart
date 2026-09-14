import 'dart:io';

import 'package:buzz/app.dart';
import 'package:buzz/features/pairing/pairing_page.dart';
import 'package:buzz/main.dart' as buzz;
import 'package:flutter_test/flutter_test.dart';
import 'package:google_mlkit_selfie_segmentation/google_mlkit_selfie_segmentation.dart';
import 'package:image/image.dart' as image;
import 'package:integration_test/integration_test.dart';
import 'package:path_provider/path_provider.dart';

void main() {
  IntegrationTestWidgetsFlutterBinding.ensureInitialized();

  testWidgets('fresh installation launches the real pairing screen', (
    tester,
  ) async {
    await buzz.runBuzzApp(const App());
    await tester.pumpAndSettle(const Duration(milliseconds: 100));
    expect(find.byType(PairingPage), findsOneWidget);
    expect(tester.takeException(), isNull);
  });

  testWidgets('native ML Kit processes an image on the simulator', (
    tester,
  ) async {
    final directory = await (await getTemporaryDirectory()).createTemp(
      'mlkit-smoke-',
    );
    final frame = File('${directory.path}/frame.png');
    final segmenter = SelfieSegmenter(mode: SegmenterMode.stream);
    try {
      final pixels = image.Image(width: 128, height: 128);
      image.fill(pixels, color: image.ColorRgb8(255, 255, 255));
      await frame.writeAsBytes(image.encodePng(pixels));
      final mask = await segmenter.processImage(
        InputImage.fromFilePath(frame.path),
      );
      expect(mask, isNotNull);
      expect(mask!.width, 128);
      expect(mask.height, 128);
      expect(mask.confidences, hasLength(mask.width * mask.height));
      expect(
        mask.confidences.every(
          (value) => value.isFinite && value >= 0 && value <= 1,
        ),
        isTrue,
      );
    } finally {
      await segmenter.close();
      await directory.delete(recursive: true);
    }
  });
}
