import Foundation
import UIKit

private enum MediaSanitizationError: Error {
  case invalidPng
  case invalidJpeg
}

enum MediaSanitizer {
  private static let pngSignature = Data([0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A])
  private static let allowedPngAncillaryChunks: Set<String> = [
    "cHRM", "gAMA", "sBIT", "sRGB", "bKGD", "hIST", "tRNS", "sPLT", "acTL", "fcTL", "fdAT",
  ]

  static func sanitizeImage(_ image: UIImage, mimeType: String) throws -> Data? {
    switch mimeType {
    case "image/png":
      guard let image = renderInSRGB(image), let encoded = image.pngData() else { return nil }
      return try scrubPng(encoded)
    case "image/jpeg":
      return try encodeJpeg(image)
    case "image/webp":
      guard let image = renderInSRGB(image), let encoded = image.pngData() else { return nil }
      return try scrubPng(encoded)
    default:
      return nil
    }
  }

  static func encodeJpeg(_ image: UIImage) throws -> Data? {
    guard
      let image = renderInSRGB(image),
      let encoded = image.jpegData(compressionQuality: 1.0)
    else {
      return nil
    }
    return try scrubJpeg(encoded)
  }

  private static func renderInSRGB(_ image: UIImage) -> UIImage? {
    guard image.size.width > 0, image.size.height > 0 else { return nil }
    let format = UIGraphicsImageRendererFormat()
    format.scale = image.scale
    format.opaque = false
    format.preferredRange = .standard
    return UIGraphicsImageRenderer(size: image.size, format: format).image { _ in
      image.draw(in: CGRect(origin: .zero, size: image.size))
    }
  }

  static func scrubPng(_ data: Data) throws -> Data {
    let data = Data(data)
    guard data.count >= pngSignature.count, data.prefix(pngSignature.count) == pngSignature else {
      throw MediaSanitizationError.invalidPng
    }

    var output = pngSignature
    var offset = pngSignature.count
    while offset < data.count {
      guard data.count - offset >= 12 else {
        throw MediaSanitizationError.invalidPng
      }

      let payloadLengthValue = try readUInt32BigEndian(data, at: offset)
      guard
        let payloadLength = Int(exactly: payloadLengthValue),
        payloadLength <= data.count - offset - 12
      else {
        throw MediaSanitizationError.invalidPng
      }
      let chunkLength = payloadLength + 12
      let typeStart = offset + 4
      let typeEnd = typeStart + 4
      let typeBytes = data[typeStart..<typeEnd]
      guard let type = String(bytes: typeBytes, encoding: .ascii) else {
        throw MediaSanitizationError.invalidPng
      }

      let isAncillary = typeBytes[typeBytes.startIndex] & 0x20 != 0
      if !isAncillary || allowedPngAncillaryChunks.contains(type) {
        output.append(data[offset..<(offset + chunkLength)])
      }

      offset += chunkLength
      if type == "IEND" {
        return output
      }
    }

    throw MediaSanitizationError.invalidPng
  }

  static func scrubJpeg(_ data: Data) throws -> Data {
    let data = Data(data)
    guard data.count >= 2, data[0] == 0xFF, data[1] == 0xD8 else {
      throw MediaSanitizationError.invalidJpeg
    }

    var output = Data([0xFF, 0xD8])
    var offset = 2
    var inScan = false
    while offset < data.count {
      if inScan, data[offset] != 0xFF {
        let nextMarker = data[offset...].firstIndex(of: 0xFF) ?? data.endIndex
        output.append(data[offset..<nextMarker])
        offset = nextMarker
        continue
      }
      guard data[offset] == 0xFF else {
        throw MediaSanitizationError.invalidJpeg
      }

      let markerStart = offset
      while offset < data.count, data[offset] == 0xFF {
        offset += 1
      }
      guard offset < data.count else {
        throw MediaSanitizationError.invalidJpeg
      }

      let marker = data[offset]
      offset += 1
      if inScan, marker == 0x00 {
        output.append(data[markerStart..<offset])
        continue
      }
      if (0xD0...0xD7).contains(marker) || marker == 0x01 {
        output.append(data[markerStart..<offset])
        continue
      }
      if marker == 0xD9 {
        output.append(data[markerStart..<offset])
        return output
      }
      guard marker != 0xD8, data.count - offset >= 2 else {
        throw MediaSanitizationError.invalidJpeg
      }

      let segmentLength = try readUInt16BigEndian(data, at: offset)
      guard segmentLength >= 2, Int(segmentLength) <= data.count - offset else {
        throw MediaSanitizationError.invalidJpeg
      }
      let segmentEnd = offset + Int(segmentLength)
      if shouldKeepJpegSegment(marker, data: data, payload: (offset + 2)..<segmentEnd) {
        output.append(data[markerStart..<segmentEnd])
      }

      offset = segmentEnd
      inScan = marker == 0xDA
    }

    throw MediaSanitizationError.invalidJpeg
  }

  private static func shouldKeepJpegSegment(
    _ marker: UInt8,
    data: Data,
    payload: Range<Int>
  ) -> Bool {
    switch marker {
    case 0xE0:
      guard
        payload.count >= 14,
        data[payload.lowerBound..<(payload.lowerBound + 5)].elementsEqual([
          0x4A, 0x46, 0x49, 0x46, 0x00,
        ])
      else {
        return false
      }
      let thumbnailWidth = Int(data[payload.lowerBound + 12])
      let thumbnailHeight = Int(data[payload.lowerBound + 13])
      return payload.count == 14 + 3 * thumbnailWidth * thumbnailHeight
    case 0xEE:
      return payload.count == 12
        && data[payload.lowerBound..<(payload.lowerBound + 5)].elementsEqual([
          0x41, 0x64, 0x6F, 0x62, 0x65,
        ])
    case 0xE1...0xED, 0xEF, 0xFE:
      return false
    default:
      return true
    }
  }

  private static func readUInt16BigEndian(_ data: Data, at offset: Int) throws -> UInt16 {
    guard data.count - offset >= 2 else {
      throw MediaSanitizationError.invalidJpeg
    }
    return UInt16(data[offset]) << 8 | UInt16(data[offset + 1])
  }

  private static func readUInt32BigEndian(_ data: Data, at offset: Int) throws -> UInt32 {
    guard data.count - offset >= 4 else {
      throw MediaSanitizationError.invalidPng
    }
    return UInt32(data[offset]) << 24 | UInt32(data[offset + 1]) << 16
      | UInt32(data[offset + 2]) << 8 | UInt32(data[offset + 3])
  }
}
