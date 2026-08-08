import 'dart:ffi';
import 'dart:io';
import 'dart:math' as math;
import 'dart:typed_data';

import 'package:buzz/features/huddles/huddle_wire.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:opus_codec_dart/opus_codec_dart.dart';

void main() {
  test('libopus encodes and decodes a 20 ms mono frame', () {
    initOpus(DynamicLibrary.open('libopus.so.0'));
    final encoder = SimpleOpusEncoder(
      application: Application.voip,
      sampleRate: huddleSampleRate,
      channels: 1,
    );
    final decoder = SimpleOpusDecoder(
      sampleRate: huddleSampleRate,
      channels: 1,
    );
    addTearDown(() {
      encoder.destroy();
      decoder.destroy();
    });

    final input = Int16List.fromList(
      List.generate(
        huddleFrameSamples,
        (sample) =>
            (math.sin(2 * math.pi * 440 * sample / huddleSampleRate) * 12000)
                .round(),
      ),
    );
    final encoded = encoder.encode(input: input);
    final decoded = decoder.decode(input: encoded);

    expect(encoded, isNotEmpty);
    expect(encoded.length, lessThan(huddleMaxFrameBytes - huddleHeaderLength));
    expect(decoded, hasLength(huddleFrameSamples));
    expect(decoded.any((sample) => sample != 0), isTrue);
  }, skip: !Platform.isLinux);
}
