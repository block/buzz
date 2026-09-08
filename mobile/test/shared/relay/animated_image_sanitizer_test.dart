import 'dart:convert';
import 'dart:typed_data';

import 'package:flutter_test/flutter_test.dart';
import 'package:buzz/shared/relay/animated_image_sanitizer.dart';

void main() {
  test('strips APNG metadata without changing animation chunks', () {
    final clean = _animatedPng(metadata: false);
    final dirty = _animatedPng(metadata: true)
      ..addAll(utf8.encode('trailing metadata'));

    expect(
      sanitizeAnimatedImageForUpload(Uint8List.fromList(dirty), 'image/png'),
      clean,
    );
  });

  test('strips identity APNG orientation but rejects display transforms', () {
    final clean = _animatedPng(metadata: false);
    expect(
      sanitizeAnimatedImageForUpload(
        Uint8List.fromList(_animatedPng(metadata: false, orientation: 1)),
        'image/png',
      ),
      clean,
    );

    for (final endian in [Endian.little, Endian.big]) {
      expect(
        () => sanitizeAnimatedImageForUpload(
          Uint8List.fromList(
            _animatedPng(
              metadata: false,
              orientation: 6,
              orientationEndian: endian,
            ),
          ),
          'image/png',
        ),
        throwsA(
          isA<FormatException>().having(
            (error) => error.message,
            'message',
            contains('orientation'),
          ),
        ),
      );
    }
  });

  test('rejects APNG ICC profiles that affect color rendering', () {
    expect(
      () => sanitizeAnimatedImageForUpload(
        Uint8List.fromList(_animatedPng(metadata: false, iccProfile: true)),
        'image/png',
      ),
      throwsA(
        isA<FormatException>().having(
          (error) => error.message,
          'message',
          contains('ICC profile'),
        ),
      ),
    );
  });

  test('strips animated WebP metadata and clears metadata flags', () {
    final clean = _animatedWebp(metadata: false);
    final dirty = _animatedWebp(metadata: true)
      ..addAll(utf8.encode('trailing metadata'));

    expect(
      sanitizeAnimatedImageForUpload(Uint8List.fromList(dirty), 'image/webp'),
      clean,
    );
  });

  test('strips identity WebP orientation but rejects display transforms', () {
    final clean = _animatedWebp(metadata: false);
    expect(
      sanitizeAnimatedImageForUpload(
        Uint8List.fromList(_animatedWebp(metadata: false, orientation: 1)),
        'image/webp',
      ),
      clean,
    );

    for (final endian in [Endian.little, Endian.big]) {
      expect(
        () => sanitizeAnimatedImageForUpload(
          Uint8List.fromList(
            _animatedWebp(
              metadata: false,
              orientation: 6,
              orientationEndian: endian,
            ),
          ),
          'image/webp',
        ),
        throwsA(
          isA<FormatException>().having(
            (error) => error.message,
            'message',
            contains('orientation'),
          ),
        ),
      );
    }
  });

  test('rejects animated WebP ICC profiles that affect color rendering', () {
    expect(
      () => sanitizeAnimatedImageForUpload(
        Uint8List.fromList(_animatedWebp(metadata: false, iccProfile: true)),
        'image/webp',
      ),
      throwsA(
        isA<FormatException>().having(
          (error) => error.message,
          'message',
          contains('ICC profile'),
        ),
      ),
    );
  });

  test('removes metadata chunks nested inside animated WebP frames', () {
    expect(
      sanitizeAnimatedImageForUpload(
        Uint8List.fromList(
          _animatedWebp(metadata: false, nestedMetadata: true),
        ),
        'image/webp',
      ),
      _animatedWebp(metadata: false),
    );
  });

  test('strips GIF metadata without changing animation blocks', () {
    final clean = _minimalGif();
    final dirty = <int>[
      ...clean.sublist(0, 19),
      ..._gifCommentExtension(),
      ..._gifApplicationExtension('XMP DataXMP', utf8.encode('<x/>')),
      ...clean.sublist(19),
      ...utf8.encode('trailing metadata'),
    ];

    expect(
      sanitizeAnimatedImageForUpload(Uint8List.fromList(dirty), 'image/gif'),
      clean,
    );
  });

  test('canonicalizes GIF loop extensions with hidden sub-blocks', () {
    final clean = _minimalGif();
    final cleanLoop = _gifLoopExtension();
    final dirty = <int>[
      ...clean.sublist(0, 19),
      ..._gifLoopExtension(extra: utf8.encode('location')),
      ...clean.sublist(19 + cleanLoop.length),
    ];

    expect(
      sanitizeAnimatedImageForUpload(Uint8List.fromList(dirty), 'image/gif'),
      clean,
    );
  });

  test('drops GIF applications with binary authentication codes', () {
    final clean = _minimalGif();
    final dirty = <int>[
      ...clean.sublist(0, 19),
      ..._gifApplicationExtensionBytes([
        ...ascii.encode('FOREIGN1'),
        0xff,
        0x80,
        0x00,
      ], utf8.encode('private')),
      ...clean.sublist(19),
    ];

    expect(
      sanitizeAnimatedImageForUpload(Uint8List.fromList(dirty), 'image/gif'),
      clean,
    );
  });

  test('removes a GIF graphic control consumed by stripped plain text', () {
    final clean = _minimalGif();
    final dirty = <int>[
      ...clean.sublist(0, 19),
      0x21,
      0xf9,
      4,
      0x09,
      0x1e,
      0,
      1,
      0,
      ..._gifPlainTextExtension(),
      ...clean.sublist(19),
    ];

    expect(
      sanitizeAnimatedImageForUpload(Uint8List.fromList(dirty), 'image/gif'),
      clean,
    );
  });

  test('keeps clean animated containers byte-identical', () {
    for (final (mimeType, bytes) in [
      ('image/png', _animatedPng(metadata: false)),
      ('image/webp', _animatedWebp(metadata: false)),
      ('image/gif', _minimalGif()),
    ]) {
      expect(
        sanitizeAnimatedImageForUpload(Uint8List.fromList(bytes), mimeType),
        bytes,
      );
    }
  });

  test('fails closed for malformed animated containers', () {
    for (final (mimeType, bytes) in [
      ('image/png', <int>[0x89, 0x50, 0x4e, 0x47]),
      ('image/webp', ascii.encode('RIFFxxxxWEBP')),
      ('image/gif', ascii.encode('GIF89a')),
    ]) {
      expect(
        () =>
            sanitizeAnimatedImageForUpload(Uint8List.fromList(bytes), mimeType),
        throwsFormatException,
      );
    }
  });
}

List<int> _pngChunk(String type, List<int> payload) {
  return [
    ..._uint32BigEndian(payload.length),
    ...ascii.encode(type),
    ...payload,
    0,
    0,
    0,
    0,
  ];
}

List<int> _animatedPng({
  required bool metadata,
  int? orientation,
  Endian orientationEndian = Endian.little,
  bool iccProfile = false,
}) {
  return [
    0x89,
    0x50,
    0x4e,
    0x47,
    0x0d,
    0x0a,
    0x1a,
    0x0a,
    ..._pngChunk('IHDR', List.filled(13, 0)),
    ..._pngChunk('acTL', [0, 0, 0, 2, 0, 0, 0, 0]),
    if (iccProfile) ..._pngChunk('iCCP', utf8.encode('profile')),
    if (orientation != null)
      ..._pngChunk(
        'eXIf',
        _exifOrientation(
          orientation,
          orientationEndian,
          includePreamble: false,
        ),
      ),
    if (metadata) ..._pngChunk('tEXt', utf8.encode('Location\u0000secret')),
    if (metadata) ..._pngChunk('pHYs', List.filled(9, 0)),
    ..._pngChunk('fcTL', List.filled(26, 0)),
    ..._pngChunk('IDAT', [1, 2, 3]),
    ..._pngChunk('fdAT', [0, 0, 0, 1, 4, 5]),
    ..._pngChunk('IEND', const []),
  ];
}

List<int> _webpChunk(String type, List<int> payload) {
  return [
    ...ascii.encode(type),
    ..._uint32LittleEndian(payload.length),
    ...payload,
    if (payload.length.isOdd) 0,
  ];
}

List<int> _animatedWebp({
  required bool metadata,
  int? orientation,
  Endian orientationEndian = Endian.little,
  bool iccProfile = false,
  bool nestedMetadata = false,
}) {
  final hasExif = metadata || orientation != null;
  final frame = <int>[
    ...List.filled(16, 0),
    ..._webpChunk('VP8 ', [1, 2, 3]),
    if (nestedMetadata) ..._webpChunk('JUNK', utf8.encode('location')),
  ];
  final chunks = <int>[
    ..._webpChunk('VP8X', [
      0x02 | (hasExif ? 0x0c : 0) | (iccProfile ? 0x20 : 0),
      0,
      0,
      0,
      0,
      0,
      0,
      0,
      0,
      0,
    ]),
    ..._webpChunk('ANIM', List.filled(6, 0)),
    if (iccProfile) ..._webpChunk('ICCP', utf8.encode('profile')),
    if (orientation != null)
      ..._webpChunk(
        'EXIF',
        _exifOrientation(orientation, orientationEndian, includePreamble: true),
      )
    else if (metadata)
      ..._webpChunk('EXIF', utf8.encode('location')),
    if (metadata) ..._webpChunk('XMP ', utf8.encode('<xmp/>')),
    if (metadata) ..._webpChunk('JUNK', utf8.encode('private')),
    ..._webpChunk('ANMF', frame),
  ];
  return [
    ...ascii.encode('RIFF'),
    ..._uint32LittleEndian(chunks.length + 4),
    ...ascii.encode('WEBP'),
    ...chunks,
  ];
}

List<int> _exifOrientation(
  int orientation,
  Endian endian, {
  required bool includePreamble,
}) {
  final bytes = BytesBuilder();
  if (includePreamble) {
    bytes
      ..add(ascii.encode('Exif'))
      ..add(const [0, 0]);
  }
  final tiff = ByteData(26);
  if (endian == Endian.little) {
    tiff.setUint8(0, 0x49);
    tiff.setUint8(1, 0x49);
  } else {
    tiff.setUint8(0, 0x4d);
    tiff.setUint8(1, 0x4d);
  }
  tiff.setUint16(2, 42, endian);
  tiff.setUint32(4, 8, endian);
  tiff.setUint16(8, 1, endian);
  tiff.setUint16(10, 0x0112, endian);
  tiff.setUint16(12, 3, endian);
  tiff.setUint32(14, 1, endian);
  tiff.setUint16(18, orientation, endian);
  tiff.setUint32(22, 0, endian);
  bytes.add(tiff.buffer.asUint8List());
  return bytes.takeBytes();
}

List<int> _minimalGif() {
  return [
    ...ascii.encode('GIF89a'),
    2,
    0,
    2,
    0,
    0x80,
    0,
    0,
    0,
    0,
    0,
    0xff,
    0xff,
    0xff,
    ..._gifLoopExtension(),
    0x21,
    0xf9,
    4,
    0,
    10,
    0,
    0,
    0,
    0x2c,
    0,
    0,
    0,
    0,
    2,
    0,
    2,
    0,
    0,
    2,
    2,
    0x44,
    1,
    0,
    0x3b,
  ];
}

List<int> _gifLoopExtension({List<int> extra = const []}) {
  return [
    0x21,
    0xff,
    11,
    ...ascii.encode('NETSCAPE2.0'),
    3,
    1,
    0,
    0,
    if (extra.isNotEmpty) ...[extra.length, ...extra],
    0,
  ];
}

List<int> _gifCommentExtension() {
  return [0x21, 0xfe, 5, ...ascii.encode('hello'), 0];
}

List<int> _gifApplicationExtension(String identifier, List<int> payload) {
  return _gifApplicationExtensionBytes(ascii.encode(identifier), payload);
}

List<int> _gifApplicationExtensionBytes(
  List<int> identifier,
  List<int> payload,
) {
  return [0x21, 0xff, 11, ...identifier, payload.length, ...payload, 0];
}

List<int> _gifPlainTextExtension() {
  return [0x21, 0x01, 12, 0, 0, 0, 0, 2, 0, 2, 0, 1, 1, 1, 0, 1, 0x78, 0];
}

List<int> _uint32BigEndian(int value) {
  return [
    value >> 24 & 0xff,
    value >> 16 & 0xff,
    value >> 8 & 0xff,
    value & 0xff,
  ];
}

List<int> _uint32LittleEndian(int value) {
  return [
    value & 0xff,
    value >> 8 & 0xff,
    value >> 16 & 0xff,
    value >> 24 & 0xff,
  ];
}
