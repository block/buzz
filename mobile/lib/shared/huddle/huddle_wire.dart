import 'package:flutter/foundation.dart';

/// Fixed audio framing contract shared with Buzz Desktop and `buzz-relay`.
///
/// Huddle audio rooms pin the first participant's protocol version. This
/// compatibility build requests the released v2 contract so it can connect to
/// relays that have not yet rolled out v3.
abstract final class HuddleWireV2 {
  static const protocolVersion = 2;
  static const sampleRateHz = 48000;
  static const channels = 1;
  static const frameSamples = 960;
  static const frameDuration = Duration(milliseconds: 20);
  static const headerLength = 8;
  static const relayPeerPrefixLength = 1;
  static const dtxFlag = 0x01;
  static const maxBinaryFrameLength = 4096;

  /// Encodes a client-to-relay frame: `header | opus payload`.
  static Uint8List encodeClientFrame(
    HuddleAudioHeader header,
    Uint8List opusPayload,
  ) {
    if (opusPayload.isEmpty) {
      throw const HuddleWireException('Opus payload must not be empty.');
    }
    final length = headerLength + opusPayload.length;
    if (length > maxBinaryFrameLength) {
      throw HuddleWireException(
        'Huddle audio frame is $length bytes; maximum is '
        '$maxBinaryFrameLength.',
      );
    }

    return Uint8List.fromList([...header.encode(), ...opusPayload]);
  }

  /// Decodes a relay-to-client frame:
  /// `peer index | header | opus payload`.
  static HuddleRemoteAudioFrame decodeRelayFrame(Uint8List bytes) {
    final minimumLength = relayPeerPrefixLength + headerLength + 1;
    if (bytes.length < minimumLength) {
      throw HuddleWireException(
        'Relay audio frame is ${bytes.length} bytes; expected at least '
        '$minimumLength.',
      );
    }
    if (bytes.length > maxBinaryFrameLength + relayPeerPrefixLength) {
      throw HuddleWireException(
        'Relay audio frame is ${bytes.length} bytes; maximum is '
        '${maxBinaryFrameLength + relayPeerPrefixLength}.',
      );
    }

    return HuddleRemoteAudioFrame(
      peerIndex: bytes[0],
      // Protocol v2 has no occupancy epoch on media frames. Keep the internal
      // field at its legacy value so the rest of the playout model remains
      // unchanged while routing by the authoritative peer index.
      epoch: 0,
      header: HuddleAudioHeader.decode(bytes, offset: relayPeerPrefixLength),
      opusPayload: Uint8List.fromList(
        bytes.sublist(relayPeerPrefixLength + headerLength),
      ),
    );
  }
}

/// The fixed eight-byte v2 header, encoded in network byte order.
///
/// Layout:
/// - bytes 0-1: wrapping sequence number (`u16`)
/// - bytes 2-5: sender media timestamp at 48 kHz (`u32`)
/// - byte 6: sender-authored audio level in dBov (`i8`, `-127...0`)
/// - byte 7: flags (`bit 0 = Opus DTX`; unknown bits are ignored)
@immutable
final class HuddleAudioHeader {
  final int sequence;
  final int timestamp48k;
  final int levelDbov;
  final int flags;

  const HuddleAudioHeader({
    required this.sequence,
    required this.timestamp48k,
    required this.levelDbov,
    required this.flags,
  }) : assert(sequence >= 0 && sequence <= 0xffff),
       assert(timestamp48k >= 0 && timestamp48k <= 0xffffffff),
       assert(levelDbov >= -127 && levelDbov <= 0),
       assert(flags >= 0 && flags <= 0xff);

  /// Builds a header while enforcing the wire ranges in release builds too.
  factory HuddleAudioHeader.checked({
    required int sequence,
    required int timestamp48k,
    required int levelDbov,
    required int flags,
  }) {
    _validateValues(sequence, timestamp48k, levelDbov, flags);
    return HuddleAudioHeader(
      sequence: sequence,
      timestamp48k: timestamp48k,
      levelDbov: levelDbov,
      flags: flags,
    );
  }

  bool get isDtx => flags & HuddleWireV2.dtxFlag != 0;

  Uint8List encode() {
    _validate();
    final bytes = Uint8List(HuddleWireV2.headerLength);
    final view = ByteData.sublistView(bytes);
    view.setUint16(0, sequence, Endian.big);
    view.setUint32(2, timestamp48k, Endian.big);
    view.setInt8(6, levelDbov);
    view.setUint8(7, flags);
    return bytes;
  }

  static HuddleAudioHeader decode(Uint8List bytes, {int offset = 0}) {
    if (offset < 0 || bytes.length - offset < HuddleWireV2.headerLength) {
      throw const HuddleWireException(
        'Huddle audio header requires eight bytes.',
      );
    }
    final view = ByteData.sublistView(
      bytes,
      offset,
      offset + HuddleWireV2.headerLength,
    );
    final rawLevel = view.getInt8(6);
    // Match the desktop contract: malformed VU telemetry becomes the silent
    // floor but never causes an otherwise valid Opus packet to be dropped.
    final level = rawLevel >= -127 && rawLevel <= 0 ? rawLevel : -127;
    return HuddleAudioHeader(
      sequence: view.getUint16(0, Endian.big),
      timestamp48k: view.getUint32(2, Endian.big),
      levelDbov: level,
      flags: view.getUint8(7),
    );
  }

  void _validate() => _validateValues(sequence, timestamp48k, levelDbov, flags);

  static void _validateValues(
    int sequence,
    int timestamp48k,
    int levelDbov,
    int flags,
  ) {
    if (sequence < 0 || sequence > 0xffff) {
      throw RangeError.range(sequence, 0, 0xffff, 'sequence');
    }
    if (timestamp48k < 0 || timestamp48k > 0xffffffff) {
      throw RangeError.range(timestamp48k, 0, 0xffffffff, 'timestamp48k');
    }
    if (levelDbov < -127 || levelDbov > 0) {
      throw RangeError.range(levelDbov, -127, 0, 'levelDbov');
    }
    if (flags < 0 || flags > 0xff) {
      throw RangeError.range(flags, 0, 0xff, 'flags');
    }
  }
}

@immutable
final class HuddleRemoteAudioFrame {
  final int peerIndex;

  /// Always zero for protocol v2, whose relay prefix contains only
  /// `peerIndex`. Retained as an internal compatibility field.
  final int epoch;
  final HuddleAudioHeader header;
  final Uint8List opusPayload;

  const HuddleRemoteAudioFrame({
    required this.peerIndex,
    required this.epoch,
    required this.header,
    required this.opusPayload,
  });
}

final class HuddleWireException implements Exception {
  final String message;

  const HuddleWireException(this.message);

  @override
  String toString() => 'HuddleWireException: $message';
}
