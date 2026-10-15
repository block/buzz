import Foundation
import ImageIO
import Testing

@testable import BuzzPushKit

@Suite("Native notification avatar decoding")
struct BuzzNotificationAvatarLoaderTests {
  @Test("Animated avatar selects the poster without fetching the animation")
  func animatedPoster() throws {
    let source =
      "https://images.example/poster.png#buzz-anim=https%3A%2F%2Fimages.example%2Fanimation.apng"
    #expect(
      BuzzNotificationAvatarLoader.posterURL(source)?.absoluteString
        == "https://images.example/poster.png")
    #expect(BuzzNotificationAvatarLoader.posterURL("file:///private/avatar.png") == nil)
  }

  @Test("Raster decoder produces a bounded PNG and rejects non-image data")
  func raster() throws {
    let bytes = try #require(
      Data(
        base64Encoded:
          "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mP8/x8AAwMCAO+aX1sAAAAASUVORK5CYII="
      ))
    let png = try BuzzNotificationAvatarLoader.decode(bytes)
    let source = try #require(CGImageSourceCreateWithData(png as CFData, nil))
    let properties = try #require(
      CGImageSourceCopyPropertiesAtIndex(source, 0, nil) as? [CFString: Any])
    #expect(properties[kCGImagePropertyPixelWidth] as? Int == 1)
    #expect(properties[kCGImagePropertyPixelHeight] as? Int == 1)
    #expect(png.count <= BuzzNotificationCache.maximumAvatarBytes)
    #expect(throws: BuzzNotificationCacheError.self) {
      try BuzzNotificationAvatarLoader.decode(Data("not an image".utf8))
    }
  }
}
