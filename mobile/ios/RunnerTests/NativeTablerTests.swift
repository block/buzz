import UIKit
import XCTest

@testable import Buzz

final class NativeTablerTests: XCTestCase {
  @MainActor
  func testFilledCircleHasOpaqueCenterAndTransparentCornersAtEachSize() throws {
    for size: CGFloat in [16, 20, 32] {
      let image = try XCTUnwrap(NativeTabler.image("circle.fill", size: size))
      XCTAssertEqual(image.size, CGSize(width: size, height: size))
      XCTAssertEqual(image.renderingMode, .alwaysTemplate)
      let bitmap = try XCTUnwrap(image.cgImage)
      var pixels = [UInt8](repeating: 0, count: bitmap.width * bitmap.height * 4)
      try pixels.withUnsafeMutableBytes { bytes in
        let context = try XCTUnwrap(CGContext(data: bytes.baseAddress,
          width: bitmap.width, height: bitmap.height, bitsPerComponent: 8,
          bytesPerRow: bitmap.width * 4, space: CGColorSpaceCreateDeviceRGB(),
          bitmapInfo: CGImageAlphaInfo.premultipliedLast.rawValue))
        context.draw(bitmap, in: CGRect(x: 0, y: 0, width: bitmap.width, height: bitmap.height))
      }
      let center = ((bitmap.height / 2) * bitmap.width + bitmap.width / 2) * 4 + 3
      XCTAssertEqual(pixels[center], 255, "Filled circle must have an opaque center")
      XCTAssertEqual(pixels[3], 0, "The icon canvas must retain transparent corners")
    }
  }
}
