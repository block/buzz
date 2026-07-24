import 'dart:convert';
import 'dart:typed_data';

const _pngSignature = <int>[0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a];
const _allowedPngAncillaryChunks = {
  'cHRM',
  'gAMA',
  'sBIT',
  'sRGB',
  'bKGD',
  'hIST',
  'tRNS',
  'sPLT',
  'acTL',
  'fcTL',
  'fdAT',
};
const _allowedWebpChunks = {'VP8 ', 'VP8L', 'VP8X', 'ALPH', 'ANIM', 'ANMF'};
const _webpMetadataFlags = 0x20 | 0x08 | 0x04;

/// Remove metadata from an animated image without decoding its frames.
///
/// Decoding through UIKit or Android Bitmap would flatten animations. These
/// structural scrubbers retain only the chunks/extensions accepted by the
/// relay and preserve animation timing, disposal, looping, and frame data.
Uint8List sanitizeAnimatedImageForUpload(Uint8List bytes, String mimeType) {
  return switch (mimeType) {
    'image/gif' => _scrubGif(bytes),
    'image/png' => _scrubPng(bytes),
    'image/webp' => _scrubWebp(bytes),
    _ => throw FormatException('Unsupported animated image type: $mimeType'),
  };
}

Uint8List _scrubPng(Uint8List bytes) {
  if (!_startsWith(bytes, _pngSignature)) {
    throw const FormatException('Invalid PNG signature');
  }

  final output = BytesBuilder(copy: false)..add(_pngSignature);
  var offset = _pngSignature.length;
  while (offset < bytes.length) {
    if (bytes.length - offset < 12) {
      throw const FormatException('Truncated PNG chunk');
    }
    final payloadLength = _readUint32BigEndian(bytes, offset);
    final chunkLength = payloadLength + 12;
    if (payloadLength > bytes.length - offset - 12) {
      throw const FormatException('Invalid PNG chunk length');
    }
    final typeStart = offset + 4;
    final type = ascii.decode(bytes.sublist(typeStart, typeStart + 4));
    if (type == 'iCCP') {
      throw const FormatException(
        'Animated PNG ICC profile cannot be removed safely',
      );
    }
    if (type == 'eXIf') {
      final orientation = _readExifOrientation(
        bytes,
        offset + 8,
        payloadLength,
      );
      if (orientation != null && orientation >= 2 && orientation <= 8) {
        throw const FormatException(
          'Animated PNG EXIF orientation cannot be removed safely',
        );
      }
    }
    final isAncillary = bytes[typeStart] & 0x20 != 0;
    if (!isAncillary || _allowedPngAncillaryChunks.contains(type)) {
      output.add(Uint8List.sublistView(bytes, offset, offset + chunkLength));
    }

    offset += chunkLength;
    if (type == 'IEND') {
      return output.takeBytes();
    }
  }

  throw const FormatException('PNG is missing IEND');
}

Uint8List _scrubWebp(Uint8List bytes) {
  if (bytes.length < 12 ||
      !_matchesAscii(bytes, 0, 'RIFF') ||
      !_matchesAscii(bytes, 8, 'WEBP')) {
    throw const FormatException('Invalid WebP signature');
  }

  final declaredLength = _readUint32LittleEndian(bytes, 4);
  final inputEnd = declaredLength + 8;
  if (inputEnd < 12 || inputEnd > bytes.length) {
    throw const FormatException('Invalid WebP container length');
  }

  final chunks = BytesBuilder(copy: false);
  var offset = 12;
  while (offset < inputEnd) {
    if (inputEnd - offset < 8) {
      throw const FormatException('Truncated WebP chunk');
    }
    final type = ascii.decode(bytes.sublist(offset, offset + 4));
    final payloadLength = _readUint32LittleEndian(bytes, offset + 4);
    final payloadStart = offset + 8;
    final paddedLength = payloadLength + (payloadLength.isOdd ? 1 : 0);
    final chunkEnd = payloadStart + paddedLength;
    if (chunkEnd > inputEnd) {
      throw const FormatException('Invalid WebP chunk length');
    }

    if (type == 'EXIF') {
      final orientation = _readExifOrientation(
        bytes,
        payloadStart,
        payloadLength,
      );
      if (orientation != null && orientation >= 2 && orientation <= 8) {
        throw const FormatException(
          'Animated WebP EXIF orientation cannot be removed safely',
        );
      }
    }
    if (type == 'ICCP') {
      throw const FormatException(
        'Animated WebP ICC profile cannot be removed safely',
      );
    }

    if (_allowedWebpChunks.contains(type)) {
      if (type == 'VP8X') {
        if (payloadLength == 0) {
          throw const FormatException('Invalid VP8X chunk');
        }
        final payload = BytesBuilder(copy: false)
          ..addByte(bytes[payloadStart] & ~_webpMetadataFlags)
          ..add(
            Uint8List.sublistView(
              bytes,
              payloadStart + 1,
              payloadStart + payloadLength,
            ),
          );
        _addWebpChunk(chunks, type, payload.takeBytes());
      } else if (type == 'ANMF') {
        _addWebpChunk(
          chunks,
          type,
          _scrubAnmfPayload(
            Uint8List.sublistView(
              bytes,
              payloadStart,
              payloadStart + payloadLength,
            ),
          ),
        );
      } else {
        _addWebpChunk(
          chunks,
          type,
          Uint8List.sublistView(
            bytes,
            payloadStart,
            payloadStart + payloadLength,
          ),
        );
      }
    }
    offset = chunkEnd;
  }

  final chunkBytes = chunks.takeBytes();
  final output = BytesBuilder(copy: false)
    ..add(ascii.encode('RIFF'))
    ..add(_uint32LittleEndian(chunkBytes.length + 4))
    ..add(ascii.encode('WEBP'))
    ..add(chunkBytes);
  return output.takeBytes();
}

void _addWebpChunk(BytesBuilder output, String type, Uint8List payload) {
  output
    ..add(ascii.encode(type))
    ..add(_uint32LittleEndian(payload.length))
    ..add(payload);
  if (payload.length.isOdd) output.addByte(0);
}

Uint8List _scrubAnmfPayload(Uint8List payload) {
  const frameHeaderLength = 16;
  if (payload.length < frameHeaderLength) {
    throw const FormatException('Invalid WebP animation frame');
  }

  final output = BytesBuilder(copy: false)
    ..add(Uint8List.sublistView(payload, 0, frameHeaderLength));
  var offset = frameHeaderLength;
  var sawAlpha = false;
  var sawImage = false;

  while (offset < payload.length) {
    if (payload.length - offset < 8) {
      throw const FormatException('Truncated WebP animation frame chunk');
    }
    final chunkLength = _readUint32LittleEndian(payload, offset + 4);
    final chunkStart = offset + 8;
    final paddedLength = chunkLength + (chunkLength.isOdd ? 1 : 0);
    final chunkEnd = chunkStart + paddedLength;
    if (chunkEnd > payload.length) {
      throw const FormatException('Invalid WebP animation frame chunk length');
    }
    final chunkPayload = Uint8List.sublistView(
      payload,
      chunkStart,
      chunkStart + chunkLength,
    );

    if (_matchesAscii(payload, offset, 'ALPH')) {
      if (sawAlpha || sawImage) {
        throw const FormatException('Invalid WebP animation frame layout');
      }
      _addWebpChunk(output, 'ALPH', chunkPayload);
      sawAlpha = true;
    } else if (_matchesAscii(payload, offset, 'VP8 ')) {
      if (sawImage) {
        throw const FormatException('Invalid WebP animation frame layout');
      }
      _addWebpChunk(output, 'VP8 ', chunkPayload);
      sawImage = true;
    } else if (_matchesAscii(payload, offset, 'VP8L')) {
      if (sawAlpha || sawImage) {
        throw const FormatException('Invalid WebP animation frame layout');
      }
      _addWebpChunk(output, 'VP8L', chunkPayload);
      sawImage = true;
    }

    offset = chunkEnd;
  }

  if (!sawImage) {
    throw const FormatException('WebP animation frame is missing image data');
  }
  return output.takeBytes();
}

int? _readExifOrientation(
  Uint8List bytes,
  int payloadStart,
  int payloadLength,
) {
  final payloadEnd = payloadStart + payloadLength;
  var tiffStart = payloadStart;
  if (payloadLength >= 6 &&
      _matchesAscii(bytes, payloadStart, 'Exif') &&
      bytes[payloadStart + 4] == 0 &&
      bytes[payloadStart + 5] == 0) {
    tiffStart += 6;
  }
  if (payloadEnd - tiffStart < 8) return null;

  final endian = switch ((bytes[tiffStart], bytes[tiffStart + 1])) {
    (0x49, 0x49) => Endian.little,
    (0x4d, 0x4d) => Endian.big,
    _ => null,
  };
  if (endian == null) return null;

  int? readUint16(int offset) {
    if (offset < tiffStart || offset + 2 > payloadEnd) return null;
    return ByteData.sublistView(bytes, offset, offset + 2).getUint16(0, endian);
  }

  int? readUint32(int offset) {
    if (offset < tiffStart || offset + 4 > payloadEnd) return null;
    return ByteData.sublistView(bytes, offset, offset + 4).getUint32(0, endian);
  }

  if (readUint16(tiffStart + 2) != 42) return null;
  final ifdOffset = readUint32(tiffStart + 4);
  if (ifdOffset == null) return null;
  final ifdStart = tiffStart + ifdOffset;
  final entryCount = readUint16(ifdStart);
  if (entryCount == null) return null;

  final entriesStart = ifdStart + 2;
  for (var index = 0; index < entryCount; index += 1) {
    final entryStart = entriesStart + index * 12;
    if (readUint16(entryStart) == 0x0112 &&
        readUint16(entryStart + 2) == 3 &&
        readUint32(entryStart + 4) == 1) {
      return readUint16(entryStart + 8);
    }
  }
  return null;
}

Uint8List _scrubGif(Uint8List bytes) {
  if (bytes.length < 13 ||
      (!_matchesAscii(bytes, 0, 'GIF87a') &&
          !_matchesAscii(bytes, 0, 'GIF89a'))) {
    throw const FormatException('Invalid GIF signature');
  }

  var offset = 13;
  final packed = bytes[10];
  if (packed & 0x80 != 0) {
    final tableLength = 3 << ((packed & 0x07) + 1);
    offset += tableLength;
    if (offset > bytes.length) {
      throw const FormatException('Truncated GIF color table');
    }
  }

  final segments = <Uint8List?>[Uint8List.sublistView(bytes, 0, offset)];
  final pendingGraphicControls = <int>[];

  while (offset < bytes.length) {
    switch (bytes[offset]) {
      case 0x2c:
        final start = offset;
        if (bytes.length - offset < 10) {
          throw const FormatException('Truncated GIF image descriptor');
        }
        final imagePacked = bytes[offset + 9];
        offset += 10;
        if (imagePacked & 0x80 != 0) {
          offset += 3 << ((imagePacked & 0x07) + 1);
          if (offset > bytes.length) {
            throw const FormatException('Truncated GIF local color table');
          }
        }
        if (offset >= bytes.length) {
          throw const FormatException('Missing GIF LZW code size');
        }
        offset = _gifSubBlocksEnd(bytes, offset + 1);
        segments.add(Uint8List.sublistView(bytes, start, offset));
        pendingGraphicControls.clear();
      case 0x21:
        final start = offset;
        if (bytes.length - offset < 2) {
          throw const FormatException('Truncated GIF extension');
        }
        final label = bytes[offset + 1];
        offset += 2;
        switch (label) {
          case 0xf9:
            if (bytes.length - offset < 6 ||
                bytes[offset] != 4 ||
                bytes[offset + 5] != 0) {
              throw const FormatException('Invalid GIF graphic control');
            }
            offset += 6;
            segments.add(Uint8List.sublistView(bytes, start, offset));
            pendingGraphicControls.add(segments.length - 1);
          case 0xff:
            if (bytes.length - offset < 12 || bytes[offset] != 11) {
              throw const FormatException('Invalid GIF application extension');
            }
            final isLoopExtension =
                _matchesAscii(bytes, offset + 1, 'NETSCAPE2.0') ||
                _matchesAscii(bytes, offset + 1, 'ANIMEXTS1.0');
            final dataStart = offset + 12;
            offset = _gifSubBlocksEnd(bytes, dataStart);
            if (isLoopExtension) {
              if (bytes.length - dataStart < 5 ||
                  bytes[dataStart] != 3 ||
                  bytes[dataStart + 1] != 1) {
                throw const FormatException('Invalid GIF loop extension');
              }
              segments
                ..add(Uint8List.sublistView(bytes, start, dataStart + 4))
                ..add(Uint8List(1));
            }
          case 0x01:
            offset = _gifSubBlocksEnd(bytes, offset);
            for (final segmentIndex in pendingGraphicControls) {
              segments[segmentIndex] = null;
            }
            pendingGraphicControls.clear();
          default:
            offset = _gifSubBlocksEnd(bytes, offset);
        }
      case 0x3b:
        segments.add(Uint8List.sublistView(bytes, offset, offset + 1));
        final output = BytesBuilder(copy: false);
        for (final segment in segments) {
          if (segment != null) output.add(segment);
        }
        return output.takeBytes();
      default:
        throw const FormatException('Invalid GIF block');
    }
  }

  throw const FormatException('GIF is missing trailer');
}

int _gifSubBlocksEnd(Uint8List bytes, int offset) {
  while (offset < bytes.length) {
    final blockLength = bytes[offset];
    offset += 1;
    if (blockLength == 0) return offset;
    offset += blockLength;
    if (offset > bytes.length) {
      throw const FormatException('Truncated GIF data block');
    }
  }
  throw const FormatException('GIF data blocks are missing a terminator');
}

bool _startsWith(Uint8List bytes, List<int> prefix) {
  if (bytes.length < prefix.length) return false;
  for (var index = 0; index < prefix.length; index += 1) {
    if (bytes[index] != prefix[index]) return false;
  }
  return true;
}

bool _matchesAscii(Uint8List bytes, int offset, String value) {
  final expected = ascii.encode(value);
  if (bytes.length - offset < expected.length) return false;
  for (var index = 0; index < expected.length; index += 1) {
    if (bytes[offset + index] != expected[index]) return false;
  }
  return true;
}

int _readUint32BigEndian(Uint8List bytes, int offset) {
  return ByteData.sublistView(bytes, offset, offset + 4).getUint32(0);
}

int _readUint32LittleEndian(Uint8List bytes, int offset) {
  return ByteData.sublistView(
    bytes,
    offset,
    offset + 4,
  ).getUint32(0, Endian.little);
}

Uint8List _uint32LittleEndian(int value) {
  return Uint8List(4)..buffer.asByteData().setUint32(0, value, Endian.little);
}
