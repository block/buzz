import 'dart:math' as math;
import 'dart:typed_data';

const huddleProtocolVersion = 2;
const huddleHeaderLength = 8;
const huddleFrameSamples = 960;
const huddleSampleRate = 48000;
const huddleDtxFlag = 0x01;
const huddleMaxFrameBytes = 4096;

class HuddleFrameHeader {
  const HuddleFrameHeader({
    required this.sequence,
    required this.timestamp48k,
    required this.levelDbov,
    required this.flags,
  });

  final int sequence;
  final int timestamp48k;
  final int levelDbov;
  final int flags;

  bool get isDtx => flags & huddleDtxFlag != 0;

  Uint8List encode() {
    final data = ByteData(huddleHeaderLength)
      ..setUint16(0, sequence & 0xffff)
      ..setUint32(2, timestamp48k & 0xffffffff)
      ..setInt8(6, levelDbov.clamp(-127, 0))
      ..setUint8(7, flags & 0xff);
    return data.buffer.asUint8List();
  }

  static ({HuddleFrameHeader header, Uint8List payload})? parse(
    Uint8List bytes,
  ) {
    if (bytes.length < huddleHeaderLength) return null;
    final data = ByteData.sublistView(bytes);
    final rawLevel = data.getInt8(6);
    return (
      header: HuddleFrameHeader(
        sequence: data.getUint16(0),
        timestamp48k: data.getUint32(2),
        levelDbov: rawLevel >= -127 && rawLevel <= 0 ? rawLevel : -127,
        flags: data.getUint8(7),
      ),
      payload: Uint8List.sublistView(bytes, huddleHeaderLength),
    );
  }
}

class HuddleSequence {
  int sequence = 0;
  int timestamp48k = 0;

  HuddleFrameHeader next({required int levelDbov, required bool isDtx}) {
    final header = HuddleFrameHeader(
      sequence: sequence,
      timestamp48k: timestamp48k,
      levelDbov: levelDbov,
      flags: isDtx ? huddleDtxFlag : 0,
    );
    sequence = (sequence + 1) & 0xffff;
    timestamp48k = (timestamp48k + huddleFrameSamples) & 0xffffffff;
    return header;
  }
}

int audioLevelDbov(Int16List pcm) {
  if (pcm.isEmpty) return -127;
  var sum = 0.0;
  for (final sample in pcm) {
    final normalized = sample / 32768.0;
    sum += normalized * normalized;
  }
  final rms = math.sqrt(sum / pcm.length);
  if (rms <= 0 || !rms.isFinite) return -127;
  return (20 * math.log(rms) / math.ln10).round().clamp(-127, 0);
}

class PcmFrameChunker {
  final List<int> _pending = [];

  List<Int16List> add(Int16List samples) {
    _pending.addAll(samples);
    final frames = <Int16List>[];
    while (_pending.length >= huddleFrameSamples) {
      frames.add(Int16List.fromList(_pending.sublist(0, huddleFrameSamples)));
      _pending.removeRange(0, huddleFrameSamples);
    }
    return frames;
  }

  void clear() => _pending.clear();
}

Int16List mixPcmFrames(Iterable<Int16List> frames) {
  final sources = frames.toList(growable: false);
  if (sources.isEmpty) return Int16List(0);
  final sampleCount = sources.map((frame) => frame.length).reduce(math.min);
  final mixed = Int16List(sampleCount);
  for (var sample = 0; sample < sampleCount; sample++) {
    var sum = 0;
    for (final source in sources) {
      sum += source[sample];
    }
    mixed[sample] = sum.clamp(-32768, 32767);
  }
  return mixed;
}
