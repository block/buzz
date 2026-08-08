import 'dart:typed_data';

import 'package:buzz/features/huddles/huddle_wire.dart';
import 'package:flutter_test/flutter_test.dart';

void main() {
  test('v2 header is big endian and round trips metadata and payload', () {
    final bytes = Uint8List.fromList([
      ...const HuddleFrameHeader(
        sequence: 0xabcd,
        timestamp48k: 0x12345678,
        levelDbov: -40,
        flags: huddleDtxFlag,
      ).encode(),
      1,
      2,
    ]);
    expect(bytes.take(8), [0xab, 0xcd, 0x12, 0x34, 0x56, 0x78, 0xd8, 1]);
    final parsed = HuddleFrameHeader.parse(bytes)!;
    expect(parsed.header.sequence, 0xabcd);
    expect(parsed.header.timestamp48k, 0x12345678);
    expect(parsed.header.levelDbov, -40);
    expect(parsed.header.isDtx, isTrue);
    expect(parsed.payload, [1, 2]);
  });

  test('rejects short headers and clamps invalid dBov telemetry', () {
    expect(HuddleFrameHeader.parse(Uint8List(7)), isNull);
    final bytes = Uint8List.fromList([0, 0, 0, 0, 0, 0, 1, 0]);
    expect(HuddleFrameHeader.parse(bytes)!.header.levelDbov, -127);
  });

  test('sequence and timestamp wrap deterministically', () {
    final sequence = HuddleSequence()
      ..sequence = 0xffff
      ..timestamp48k = 0xfffffc40;
    final first = sequence.next(levelDbov: -1, isDtx: false);
    final second = sequence.next(levelDbov: -127, isDtx: true);
    expect(first.sequence, 0xffff);
    expect(first.timestamp48k, 0xfffffc40);
    expect(second.sequence, 0);
    expect(second.timestamp48k, 0);
    expect(second.isDtx, isTrue);
  });

  test('PCM chunker emits exact 20 ms frames and retains remainder', () {
    final chunker = PcmFrameChunker();
    expect(chunker.add(Int16List(959)), isEmpty);
    final frames = chunker.add(Int16List(962));
    expect(frames, hasLength(2));
    expect(frames.every((frame) => frame.length == 960), isTrue);
    expect(chunker.add(Int16List(99)), isEmpty);
  });

  test('audio level uses canonical silence and full-scale bounds', () {
    expect(audioLevelDbov(Int16List(960)), -127);
    expect(audioLevelDbov(Int16List.fromList(List.filled(960, 32767))), 0);
  });

  test('PCM mixer combines peers and clamps overflow', () {
    final mixed = mixPcmFrames([
      Int16List.fromList([1000, 30000, -30000]),
      Int16List.fromList([2000, 10000, -10000]),
    ]);
    expect(mixed, [3000, 32767, -32768]);
  });
}
