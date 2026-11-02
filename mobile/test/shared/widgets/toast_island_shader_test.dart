import 'dart:ui' as ui;

import 'package:flutter_test/flutter_test.dart';

void main() {
  TestWidgetsFlutterBinding.ensureInitialized();

  test('settled island shader paints only the exact toast capsule', () async {
    final program = await ui.FragmentProgram.fromAsset(
      'shaders/toast_island.frag',
    );
    final shader = program.fragmentShader();
    addTearDown(shader.dispose);
    // Island bottom 48, toast top 68: the resting 20-point separation.
    final uniforms = <double>[100, 30, 60, 18, 100, 98, 90, 30, 0, 0.25];
    for (var i = 0; i < uniforms.length; i++) {
      shader.setFloat(i, uniforms[i]);
    }
    final recorder = ui.PictureRecorder();
    final canvas = ui.Canvas(recorder);
    canvas.drawRect(
      const ui.Rect.fromLTWH(0, 0, 200, 160),
      ui.Paint()..shader = shader,
    );
    final picture = recorder.endRecording();
    final image = await picture.toImage(200, 160);
    final pixels = (await image.toByteData())!;
    int alpha(int x, int y) => pixels.getUint8((y * 200 + x) * 4 + 3);
    // No synthetic hardware island, and no hump above the flat toast top.
    expect(alpha(100, 30), 0);
    for (var x = 50; x <= 150; x += 10) {
      expect(alpha(x, 66), 0);
      expect(alpha(x, 70), 255);
    }
    expect(alpha(100, 98), 255);
    image.dispose();
    picture.dispose();
  });
}
