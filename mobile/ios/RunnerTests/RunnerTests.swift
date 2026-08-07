import AVFoundation
import Flutter
import UIKit
import XCTest

@testable import Buzz

class RunnerTests: XCTestCase {

  func testRelativeTrackInsertionTimesPreserveAudioDelay() {
    let times = AppDelegate.relativeTrackInsertionTimes(
      videoStart: CMTime(seconds: 1, preferredTimescale: 600),
      audioStart: CMTime(seconds: 1.5, preferredTimescale: 600)
    )

    XCTAssertEqual(CMTimeCompare(times.video, .zero), 0)
    XCTAssertEqual(
      CMTimeCompare(
        times.audio ?? .invalid,
        CMTime(seconds: 0.5, preferredTimescale: 600)
      ),
      0
    )
  }

  func testRelativeTrackInsertionTimesPreserveVideoDelay() {
    let times = AppDelegate.relativeTrackInsertionTimes(
      videoStart: CMTime(seconds: 2, preferredTimescale: 600),
      audioStart: CMTime(seconds: 1, preferredTimescale: 600)
    )

    XCTAssertEqual(
      CMTimeCompare(
        times.video,
        CMTime(seconds: 1, preferredTimescale: 600)
      ),
      0
    )
    XCTAssertEqual(CMTimeCompare(times.audio ?? .invalid, .zero), 0)
  }

  func testRelativeTrackInsertionTimesZeroBasesVideoWithoutAudio() {
    let times = AppDelegate.relativeTrackInsertionTimes(
      videoStart: CMTime(seconds: 3, preferredTimescale: 600),
      audioStart: nil
    )

    XCTAssertEqual(CMTimeCompare(times.video, .zero), 0)
    XCTAssertNil(times.audio)
  }

  @MainActor
  func testExpandedAttachmentSurfaceDismissesKeyboard() {
    let window = KeyboardDismissalSpyWindow()

    NativeAttachmentExpandedSurfaceBehavior.dismissKeyboard(in: window)

    XCTAssertTrue(window.didForceEndEditing)
  }

  func testExpandedAttachmentSurfaceMeasuresKeyboardOverlap() {
    XCTAssertEqual(
      NativeAttachmentExpandedSurfaceBehavior.keyboardOverlap(
        containerBounds: CGRect(x: 0, y: 0, width: 390, height: 844),
        keyboardLayoutFrame: CGRect(
          x: 0,
          y: 544,
          width: 390,
          height: 300
        )
      ),
      300
    )
    XCTAssertEqual(
      NativeAttachmentExpandedSurfaceBehavior.keyboardOverlap(
        containerBounds: CGRect(x: 0, y: 0, width: 390, height: 844),
        keyboardLayoutFrame: CGRect(x: 0, y: 844, width: 390, height: 0)
      ),
      0
    )
  }

  func testAttachmentMenuReturnsToKeyboardDismissedAnchor() {
    let anchorBounds = CGRect(x: 0, y: 0, width: 44, height: 44)

    XCTAssertEqual(
      NativeAttachmentPopoverAnchorLayout.sourceRect(
        anchorBounds: anchorBounds,
        keyboardDismissalOffset: 300,
        isExpanded: true
      ),
      anchorBounds.offsetBy(dx: 0, dy: 340)
    )
    XCTAssertEqual(
      NativeAttachmentPopoverAnchorLayout.sourceRect(
        anchorBounds: anchorBounds,
        keyboardDismissalOffset: 300,
        isExpanded: false
      ),
      anchorBounds.offsetBy(dx: 0, dy: 300)
    )
  }

  func testAttachmentMenuKeepsKeyboardWhenMenuFitsAboveTrigger() {
    XCTAssertEqual(
      NativeAttachmentPopoverPresentationLayout.keyboardDismissalOffset(
        sourceRect: CGRect(x: 320, y: 480, width: 44, height: 44),
        containerBounds: CGRect(x: 0, y: 0, width: 390, height: 844),
        safeAreaInsets: UIEdgeInsets(top: 59, left: 0, bottom: 34, right: 0),
        keyboardLayoutFrame: CGRect(
          x: 0,
          y: 544,
          width: 390,
          height: 300
        ),
        menuHeight: NativeAttachmentMenuLayout.size(
          compatibleWith: UITraitCollection(
            preferredContentSizeCategory: .large
          )
        ).height
      ),
      0
    )
  }

  func testAttachmentMenuDismissesKeyboardAndRepositionsInCompactHeight() {
    let sourceRect = CGRect(x: 760, y: 168, width: 44, height: 44)
    let keyboardDismissalOffset =
      NativeAttachmentPopoverPresentationLayout.keyboardDismissalOffset(
        sourceRect: sourceRect,
        containerBounds: CGRect(x: 0, y: 0, width: 844, height: 390),
        safeAreaInsets: UIEdgeInsets(top: 0, left: 59, bottom: 21, right: 59),
        keyboardLayoutFrame: CGRect(
          x: 0,
          y: 228,
          width: 844,
          height: 162
        ),
        menuHeight: NativeAttachmentMenuLayout.size(
          compatibleWith: UITraitCollection(
            preferredContentSizeCategory: .large
          )
        ).height
      )

    XCTAssertEqual(keyboardDismissalOffset, 162)
    XCTAssertEqual(
      NativeAttachmentPopoverPresentationLayout.sourceRect(
        sourceRect,
        keyboardDismissalOffset: keyboardDismissalOffset
      ),
      sourceRect.offsetBy(dx: 0, dy: 162)
    )
  }

  func testAttachmentMenuDoesNotMoveWithoutSoftwareKeyboard() {
    XCTAssertEqual(
      NativeAttachmentPopoverPresentationLayout.keyboardDismissalOffset(
        sourceRect: CGRect(x: 760, y: 168, width: 44, height: 44),
        containerBounds: CGRect(x: 0, y: 0, width: 844, height: 390),
        safeAreaInsets: UIEdgeInsets(top: 0, left: 59, bottom: 21, right: 59),
        keyboardLayoutFrame: CGRect(x: 0, y: 390, width: 844, height: 0),
        menuHeight: NativeAttachmentMenuLayout.size(
          compatibleWith: UITraitCollection(
            preferredContentSizeCategory: .large
          )
        ).height
      ),
      0
    )
  }

  func testNativeAttachmentMenuUsesRoomyRowsAndInsets() {
    let traits = UITraitCollection(preferredContentSizeCategory: .large)
    let size = NativeAttachmentMenuLayout.size(compatibleWith: traits)

    XCTAssertEqual(size.width, 216)
    XCTAssertEqual(size.height, 264)
    XCTAssertEqual(NativeAttachmentMenuLayout.contentPadding, 16)
    XCTAssertEqual(
      NativeAttachmentMenuLayout.itemHeight(compatibleWith: traits),
      52
    )
    XCTAssertEqual(NativeAttachmentMenuLayout.itemSpacing, 8)
    XCTAssertEqual(NativeAttachmentMenuLayout.labelTextStyle, .title3)
  }

  func testNativeAttachmentMenuUsesInterAndSharedPopoverChrome() {
    let font = NativeAttachmentMenuTypography.font(
      forTextStyle: NativeAttachmentMenuLayout.labelTextStyle
    )
    var didSelect = false
    let button = makeNativeAttachmentMenuButton(
      title: "Photos",
      symbol: "photo",
      action: { didSelect = true }
    )
    let titleLabel = button.subviews.compactMap { $0 as? UILabel }.first

    XCTAssertTrue(font.fontName.hasPrefix("Inter"))
    XCTAssertTrue(titleLabel?.font.fontName.hasPrefix("Inter") == true)
    XCTAssertEqual(NativeAttachmentPopoverStyle.cornerRadius, 20)
    XCTAssertEqual(NativeAttachmentPopoverStyle.borderWidth, 1)
    XCTAssertEqual(NativeAttachmentPopoverStyle.shadowOpacity, 0.18)

    button.sendActions(for: .primaryActionTriggered)
    XCTAssertTrue(didSelect)
  }

  func testNativeAttachmentMenuGrowsAndScrollsForAccessibilityText() {
    let traits = UITraitCollection(
      preferredContentSizeCategory: .accessibilityExtraExtraExtraLarge
    )
    let itemHeight = NativeAttachmentMenuLayout.itemHeight(
      compatibleWith: traits
    )
    let contentHeight = NativeAttachmentMenuLayout.contentHeight(
      compatibleWith: traits
    )
    let size = NativeAttachmentMenuLayout.size(compatibleWith: traits)

    XCTAssertGreaterThan(itemHeight, 52)
    XCTAssertGreaterThan(contentHeight, 264)
    XCTAssertEqual(
      size.height,
      min(contentHeight, NativeAttachmentMenuLayout.maximumHeight)
    )
    XCTAssertLessThanOrEqual(
      size.height,
      NativeAttachmentMenuLayout.maximumHeight
    )
  }

  func testDynamicIslandQrScannerRecognizesTallSafeAreas() {
    for safeAreaTopInset in [51, 59, 62] {
      XCTAssertTrue(
        AppDelegate.usesDynamicIslandQrScannerPortal(
          safeAreaTopInset: CGFloat(safeAreaTopInset)
        ),
        "\(safeAreaTopInset)"
      )
    }
  }

  func testDynamicIslandQrScannerRejectsStandardSafeAreas() {
    for safeAreaTopInset in [0, 44, 47, 50] {
      XCTAssertFalse(
        AppDelegate.usesDynamicIslandQrScannerPortal(
          safeAreaTopInset: CGFloat(safeAreaTopInset)
        ),
        "\(safeAreaTopInset)"
      )
    }
  }

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

  func testSanitizePngSupportsDataSlices() throws {
    let fixture = try fixtureData(named: "UIKitEncoded", extension: "png")
    let padded = Data([0x00]) + fixture
    let slice = padded.dropFirst()
    XCTAssertNotEqual(slice.startIndex, 0)

    let sanitized = try MediaSanitizer.scrubPng(slice)

    try assertMatchesRelayImageMetadataPolicy(sanitized, mimeType: "image/png")
    XCTAssertNotNil(UIImage(data: sanitized))
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

  func testSanitizeJpegSupportsDataSlices() throws {
    let fixture = try fixtureData(named: "UIKitEncoded", extension: "jpg")
    let padded = Data([0x00]) + fixture
    let slice = padded.dropFirst()
    XCTAssertNotEqual(slice.startIndex, 0)

    let sanitized = try MediaSanitizer.scrubJpeg(slice)

    try assertMatchesRelayImageMetadataPolicy(sanitized, mimeType: "image/jpeg")
    XCTAssertNotNil(UIImage(data: sanitized))
  }

  func testEncodeJpegScrubsUIKitOutput() throws {
    let fixture = try fixtureData(named: "UIKitEncoded", extension: "jpg")
    let image = try XCTUnwrap(UIImage(data: fixture))

    let encoded = try XCTUnwrap(MediaSanitizer.encodeJpeg(image))

    try assertMatchesRelayImageMetadataPolicy(encoded, mimeType: "image/jpeg")
    XCTAssertNotNil(UIImage(data: encoded))
  }

  func testSanitizeDisplayP3ImagePreservesRenderedColorInSRGB() throws {
    let image = try displayP3Image(red: 0.9, green: 0.2, blue: 0.1)
    let expectedColor = try sRGBPixel(from: image)
    let mimeTypesAndAccuracy: [(mimeType: String, accuracy: UInt8)] = [
      ("image/png", 0), ("image/jpeg", 1),
    ]

    for (mimeType, accuracy) in mimeTypesAndAccuracy {
      let sanitized = try XCTUnwrap(
        MediaSanitizer.sanitizeImage(image, mimeType: mimeType),
        "Failed to sanitize Display-P3 image as \(mimeType)"
      )

      try assertMatchesRelayImageMetadataPolicy(sanitized, mimeType: mimeType)
      let decoded = try XCTUnwrap(UIImage(data: sanitized))
      XCTAssertEqual(
        decoded.cgImage?.colorSpace?.name,
        CGColorSpace(name: CGColorSpace.sRGB)?.name
      )
      let actualColor = try sRGBPixel(from: decoded)
      XCTAssertEqual(actualColor.count, expectedColor.count)
      for (actual, expected) in zip(actualColor, expectedColor) {
        XCTAssertLessThanOrEqual(
          actual > expected ? actual - expected : expected - actual,
          accuracy
        )
      }
    }
  }

  private func displayP3Image(red: CGFloat, green: CGFloat, blue: CGFloat) throws -> UIImage {
    let colorSpace = try XCTUnwrap(CGColorSpace(name: CGColorSpace.displayP3))
    let bitmapInfo = CGBitmapInfo(rawValue: CGImageAlphaInfo.premultipliedLast.rawValue)
    let context = try XCTUnwrap(
      CGContext(
        data: nil,
        width: 1,
        height: 1,
        bitsPerComponent: 8,
        bytesPerRow: 4,
        space: colorSpace,
        bitmapInfo: bitmapInfo.rawValue
      )
    )
    context.setFillColor(
      try XCTUnwrap(CGColor(colorSpace: colorSpace, components: [red, green, blue, 1]))
    )
    context.fill(CGRect(x: 0, y: 0, width: 1, height: 1))
    return UIImage(cgImage: try XCTUnwrap(context.makeImage()))
  }

  private func sRGBPixel(from image: UIImage) throws -> [UInt8] {
    let colorSpace = try XCTUnwrap(CGColorSpace(name: CGColorSpace.sRGB))
    var bytes = [UInt8](repeating: 0, count: 4)
    let bitmapInfo = CGBitmapInfo(rawValue: CGImageAlphaInfo.premultipliedLast.rawValue)
    let context = try bytes.withUnsafeMutableBytes { bytes in
      try XCTUnwrap(
        CGContext(
          data: bytes.baseAddress,
          width: 1,
          height: 1,
          bitsPerComponent: 8,
          bytesPerRow: 4,
          space: colorSpace,
          bitmapInfo: bitmapInfo.rawValue
        )
      )
    }
    context.interpolationQuality = .none
    context.draw(try XCTUnwrap(image.cgImage), in: CGRect(x: 0, y: 0, width: 1, height: 1))
    return bytes
  }

  private func fixtureData(named name: String, extension fileExtension: String) throws -> Data {
    let url = try XCTUnwrap(
      Bundle(for: RunnerTests.self).url(forResource: name, withExtension: fileExtension))
    return try Data(contentsOf: url)
  }
}

private final class KeyboardDismissalSpyWindow: UIWindow {
  private(set) var didForceEndEditing = false

  override func endEditing(_ force: Bool) -> Bool {
    didForceEndEditing = force
    return true
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
