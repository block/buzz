import Flutter
import UIKit
import XCTest

@testable import Buzz

class RunnerTests: XCTestCase {

  func testClipboardImageDataPrefersOriginalPngBytes() throws {
    let pasteboard = try XCTUnwrap(
      UIPasteboard(name: UIPasteboard.Name(UUID().uuidString), create: true)
    )
    defer { UIPasteboard.remove(withName: pasteboard.name) }
    let pngData = Data([0x89, 0x50, 0x4E, 0x47])
    let jpegData = Data([0xFF, 0xD8, 0xFF])
    pasteboard.setItems([
      ["public.png": pngData, "public.jpeg": jpegData]
    ])

    XCTAssertEqual(AppDelegate.clipboardImageData(from: pasteboard), pngData)
  }

  func testClipboardImageDataPreservesOriginalWebPBytesForValidation() throws {
    let pasteboard = try XCTUnwrap(
      UIPasteboard(name: UIPasteboard.Name(UUID().uuidString), create: true)
    )
    defer { UIPasteboard.remove(withName: pasteboard.name) }
    let webPData = Data("RIFFxxxxWEBP".utf8)
    pasteboard.setData(webPData, forPasteboardType: "org.webmproject.webp")

    XCTAssertEqual(AppDelegate.clipboardImageData(from: pasteboard), webPData)
  }

  func testClipboardImageDataPreservesOriginalGifBytesForValidation() throws {
    let pasteboard = try XCTUnwrap(
      UIPasteboard(name: UIPasteboard.Name(UUID().uuidString), create: true)
    )
    defer { UIPasteboard.remove(withName: pasteboard.name) }
    let gifData = Data("GIF89a".utf8)
    pasteboard.setData(gifData, forPasteboardType: "com.compuserve.gif")

    XCTAssertEqual(AppDelegate.clipboardImageData(from: pasteboard), gifData)
  }

  func testClipboardImageDataReturnsNilWithoutAnImage() throws {
    let pasteboard = try XCTUnwrap(
      UIPasteboard(name: UIPasteboard.Name(UUID().uuidString), create: true)
    )
    defer { UIPasteboard.remove(withName: pasteboard.name) }
    pasteboard.string = "text only"

    XCTAssertNil(AppDelegate.clipboardImageData(from: pasteboard))
  }

  func testSanitizePngRemovesUIKitMetadataChunks() throws {
    let fixture = try fixtureData(named: "UIKitEncoded", extension: "png")
    XCTAssertEqual(
      try pngChunkTypes(fixture),
      [
        "IHDR", "sRGB", "eXIf", "pHYs", "iDOT", "IDAT", "IDAT", "IEND",
      ])

    let sanitized = try MediaSanitizer.scrubPng(fixture)

    XCTAssertEqual(
      try pngChunkTypes(sanitized),
      [
        "IHDR", "sRGB", "IDAT", "IDAT", "IEND",
      ])
    try assertMatchesRelayImageMetadataPolicy(sanitized, mimeType: "image/png")
    XCTAssertNotNil(UIImage(data: sanitized))

    var withTrailingPayload = fixture
    withTrailingPayload.append(Data("hidden location".utf8))
    let scrubbedTrailingPayload = try MediaSanitizer.scrubPng(withTrailingPayload)
    XCTAssertEqual(scrubbedTrailingPayload, sanitized)
  }

  func testSanitizeJpegRemovesUIKitMetadataSegments() throws {
    let fixture = try fixtureData(named: "UIKitEncoded", extension: "jpg")
    XCTAssertEqual(try jpegMetadataMarkers(fixture), [0xE0, 0xE1, 0xED])

    let sanitized = try MediaSanitizer.scrubJpeg(fixture)

    XCTAssertEqual(try jpegMetadataMarkers(sanitized), [0xE0])
    try assertMatchesRelayImageMetadataPolicy(sanitized, mimeType: "image/jpeg")
    XCTAssertNotNil(UIImage(data: sanitized))

    var withTrailingPayload = fixture
    withTrailingPayload.append(Data("hidden location".utf8))
    let scrubbedTrailingPayload = try MediaSanitizer.scrubJpeg(withTrailingPayload)
    XCTAssertEqual(scrubbedTrailingPayload, sanitized)
  }

  func testEncodeJpegScrubsUIKitOutput() throws {
    let fixture = try fixtureData(named: "UIKitEncoded", extension: "jpg")
    let image = try XCTUnwrap(UIImage(data: fixture))

    let encoded = try XCTUnwrap(MediaSanitizer.encodeJpeg(image))

    try assertMatchesRelayImageMetadataPolicy(encoded, mimeType: "image/jpeg")
    XCTAssertNotNil(UIImage(data: encoded))
  }

  private func fixtureData(named name: String, extension fileExtension: String) throws -> Data {
    let url = try XCTUnwrap(
      Bundle(for: RunnerTests.self).url(forResource: name, withExtension: fileExtension))
    return try Data(contentsOf: url)
  }
}

private enum RelayImagePolicyError: Error {
  case invalidPng
  case invalidJpeg
  case metadataForbidden
}

private let pngSignature = Data([0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A])
private let allowedPngAncillaryChunks: Set<String> = [
  "cHRM", "gAMA", "sBIT", "sRGB", "bKGD", "hIST", "tRNS", "sPLT", "acTL", "fcTL", "fdAT",
]

private func assertMatchesRelayImageMetadataPolicy(_ data: Data, mimeType: String) throws {
  switch mimeType {
  case "image/png":
    guard data.count >= pngSignature.count, data.prefix(pngSignature.count) == pngSignature else {
      throw RelayImagePolicyError.invalidPng
    }
    var offset = pngSignature.count
    while offset < data.count {
      guard data.count - offset >= 12 else { throw RelayImagePolicyError.invalidPng }
      let payloadLength = Int(try readUInt32BigEndian(data, at: offset))
      guard payloadLength <= data.count - offset - 12 else {
        throw RelayImagePolicyError.invalidPng
      }
      let typeBytes = data[(offset + 4)..<(offset + 8)]
      guard let type = String(bytes: typeBytes, encoding: .ascii) else {
        throw RelayImagePolicyError.invalidPng
      }
      let chunkEnd = offset + payloadLength + 12
      let isAncillary = typeBytes[typeBytes.startIndex] & 0x20 != 0
      if isAncillary, !allowedPngAncillaryChunks.contains(type) {
        throw RelayImagePolicyError.metadataForbidden
      }
      offset = chunkEnd
      if type == "IEND" {
        guard offset == data.count else { throw RelayImagePolicyError.metadataForbidden }
        return
      }
    }
    throw RelayImagePolicyError.invalidPng
  case "image/jpeg":
    guard data.count >= 2, data[0] == 0xFF, data[1] == 0xD8 else {
      throw RelayImagePolicyError.invalidJpeg
    }
    var offset = 2
    var inScan = false
    while offset < data.count {
      if inScan, data[offset] != 0xFF {
        offset += 1
        continue
      }
      guard data[offset] == 0xFF else { throw RelayImagePolicyError.invalidJpeg }
      while offset < data.count, data[offset] == 0xFF { offset += 1 }
      guard offset < data.count else { throw RelayImagePolicyError.invalidJpeg }
      let marker = data[offset]
      offset += 1
      if inScan, marker == 0x00 { continue }
      if (0xD0...0xD7).contains(marker) || marker == 0x01 { continue }
      if marker == 0xD9 {
        guard offset == data.count else { throw RelayImagePolicyError.metadataForbidden }
        return
      }
      guard marker != 0xD8, data.count - offset >= 2 else {
        throw RelayImagePolicyError.invalidJpeg
      }
      let length = Int(try readUInt16BigEndian(data, at: offset))
      guard length >= 2, length <= data.count - offset else {
        throw RelayImagePolicyError.invalidJpeg
      }
      let payload = (offset + 2)..<(offset + length)
      if marker == 0xE0 {
        guard
          payload.count >= 14,
          data[payload.lowerBound..<(payload.lowerBound + 5)].elementsEqual([
            0x4A, 0x46, 0x49, 0x46, 0x00,
          ]),
          payload.count
            == 14 + 3 * Int(data[payload.lowerBound + 12]) * Int(data[payload.lowerBound + 13])
        else {
          throw RelayImagePolicyError.metadataForbidden
        }
      } else if marker == 0xEE {
        guard
          payload.count == 12,
          data[payload.lowerBound..<(payload.lowerBound + 5)].elementsEqual([
            0x41, 0x64, 0x6F, 0x62, 0x65,
          ])
        else {
          throw RelayImagePolicyError.metadataForbidden
        }
      } else if (0xE1...0xED).contains(marker) || marker == 0xEF || marker == 0xFE {
        throw RelayImagePolicyError.metadataForbidden
      }
      offset += length
      inScan = marker == 0xDA
    }
    throw RelayImagePolicyError.invalidJpeg
  default:
    XCTFail("Unsupported test MIME type: \(mimeType)")
  }
}

private func pngChunkTypes(_ data: Data) throws -> [String] {
  guard data.count >= pngSignature.count, data.prefix(pngSignature.count) == pngSignature else {
    throw RelayImagePolicyError.invalidPng
  }
  var result: [String] = []
  var offset = pngSignature.count
  while offset < data.count {
    guard data.count - offset >= 12 else { throw RelayImagePolicyError.invalidPng }
    let payloadLength = Int(try readUInt32BigEndian(data, at: offset))
    guard payloadLength <= data.count - offset - 12 else { throw RelayImagePolicyError.invalidPng }
    guard let type = String(bytes: data[(offset + 4)..<(offset + 8)], encoding: .ascii) else {
      throw RelayImagePolicyError.invalidPng
    }
    result.append(type)
    offset += payloadLength + 12
    if type == "IEND" { return result }
  }
  throw RelayImagePolicyError.invalidPng
}

private func jpegMetadataMarkers(_ data: Data) throws -> [UInt8] {
  guard data.count >= 2, data[0] == 0xFF, data[1] == 0xD8 else {
    throw RelayImagePolicyError.invalidJpeg
  }
  var result: [UInt8] = []
  var offset = 2
  var inScan = false
  while offset < data.count {
    if inScan, data[offset] != 0xFF {
      offset += 1
      continue
    }
    guard data[offset] == 0xFF else { throw RelayImagePolicyError.invalidJpeg }
    while offset < data.count, data[offset] == 0xFF { offset += 1 }
    guard offset < data.count else { throw RelayImagePolicyError.invalidJpeg }
    let marker = data[offset]
    offset += 1
    if inScan, marker == 0x00 { continue }
    if (0xD0...0xD7).contains(marker) || marker == 0x01 { continue }
    if marker == 0xD9 { return result }
    guard marker != 0xD8, data.count - offset >= 2 else {
      throw RelayImagePolicyError.invalidJpeg
    }
    let length = Int(try readUInt16BigEndian(data, at: offset))
    guard length >= 2, length <= data.count - offset else {
      throw RelayImagePolicyError.invalidJpeg
    }
    if (0xE0...0xEF).contains(marker) || marker == 0xFE {
      result.append(marker)
    }
    offset += length
    inScan = marker == 0xDA
  }
  throw RelayImagePolicyError.invalidJpeg
}

private func readUInt16BigEndian(_ data: Data, at offset: Int) throws -> UInt16 {
  guard data.count - offset >= 2 else { throw RelayImagePolicyError.invalidJpeg }
  return UInt16(data[offset]) << 8 | UInt16(data[offset + 1])
}

private func readUInt32BigEndian(_ data: Data, at offset: Int) throws -> UInt32 {
  guard data.count - offset >= 4 else { throw RelayImagePolicyError.invalidPng }
  return UInt32(data[offset]) << 24 | UInt32(data[offset + 1]) << 16
    | UInt32(data[offset + 2]) << 8 | UInt32(data[offset + 3])
}
