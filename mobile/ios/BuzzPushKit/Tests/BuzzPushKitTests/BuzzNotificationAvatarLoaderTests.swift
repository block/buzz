import Foundation
import ImageIO
import Testing

@testable import BuzzPushKit

@Suite("Native notification avatar decoding")
struct BuzzNotificationAvatarLoaderTests {
  @Test("Avatar proofs preserve IPv6 brackets and explicit ports")
  func signedServerAuthority() throws {
    let key = String(repeating: "0", count: 63) + "1"
    let account = "79be667ef9dcbbac55a06295ce870b07029bfcdb2dce28d959f2815b16f81798"
    for authority in ["[::1]:3000", "[2001:db8::1]", "relay.example:443", "relay.example"] {
      let url = try #require(URL(string: "https://\(authority)/media/avatar.png"))
      let header = try BuzzNotificationAvatarLoader.authorization(url: url, key: key, account: account)
      let data = try #require(Data(base64Encoded: String(header.dropFirst(6))))
      let event = try JSONDecoder().decode(VerifiedNostrEvent.self, from: data)
      #expect(event.tags.contains(["server", authority]))
      #expect(event.kind == 24242)
      #expect(event.hasValidIDAndSignature())
    }
  }

  @Test("Relay media uses canonical hosts and default ports without trusting other origins")
  func mediaOrigins() throws {
    let hash = String(repeating: "a", count: 64)
    for (relay, media, expected) in [
      ("wss://relay.example:443", "https://relay.example", true),
      ("wss://relay.example", "https://RELAY.EXAMPLE:443", true),
      ("ws://relay.example:80", "http://RELAY.EXAMPLE", true),
      ("wss://relay.example:8443", "https://relay.example:8443", true),
      ("wss://relay.example", "https://relay.example:8443", false),
      ("wss://relay.example", "http://relay.example", false),
      ("wss://relay.example", "https://other.example", false),
    ] {
      let scope = try #require(
        BuzzNotificationCacheScope(
          community: PushLeaseCommunity(
            id: "community", name: "Test", relayUrl: relay, pubkey: "account", policies: [],
            notificationSessionID: "session")))
      let url = try #require(URL(string: "\(media)/media/\(hash).png"))
      #expect((BuzzNotificationAvatarLoader.thumbnailURL(url, scope: scope) != nil) == expected)
    }
  }

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
