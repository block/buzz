import XCTest

@testable import Buzz

/// The Swift parser must accept exactly what `parseInviteDeepLink` accepts;
/// these cases mirror `test/shared/deeplink/deep_link_test.dart`.
final class BuzzInviteLinkTests: XCTestCase {
  private func parse(_ string: String, debug: Bool = false) -> BuzzInviteLink? {
    BuzzInviteLink.parse(URL(string: string)!, allowInsecureLocalhost: debug)
  }

  func testHttpsInviteResolvesToSecureRelay() throws {
    let link = try XCTUnwrap(parse("https://relay.example.com/invite/abc123"))
    XCTAssertEqual(link.relayURL, "wss://relay.example.com")
    XCTAssertEqual(link.code, "abc123")
    XCTAssertNil(link.policyReceipt)
    XCTAssertEqual(link.host, "relay.example.com")
    XCTAssertEqual(link.communityName, "relay")
    XCTAssertEqual(link.relayInfoURL?.absoluteString, "https://relay.example.com/")
  }

  func testHttpsInviteKeepsExplicitPort() throws {
    let link = try XCTUnwrap(parse("https://Relay.Example.com:8443/invite/code"))
    XCTAssertEqual(link.relayURL, "wss://relay.example.com:8443")
    XCTAssertEqual(link.host, "relay.example.com:8443")
  }

  func testRejectsNonInvitePathsCredentialsAndFragments() {
    XCTAssertNil(parse("https://relay.example.com/invite"))
    XCTAssertNil(parse("https://relay.example.com/invite/abc/"))
    XCTAssertNil(parse("https://relay.example.com/other/abc"))
    XCTAssertNil(parse("https://user:pw@relay.example.com/invite/abc"))
    XCTAssertNil(parse("https://relay.example.com/invite/abc#frag"))
    XCTAssertNil(parse("ftp://relay.example.com/invite/abc"))
  }

  func testPlaintextLocalhostOnlyInDebug() {
    XCTAssertNil(parse("http://localhost:3000/invite/abc"))
    let link = parse("http://localhost:3000/invite/abc", debug: true)
    XCTAssertEqual(link?.relayURL, "ws://localhost:3000")
    XCTAssertEqual(link?.communityName, "Local Dev")
    XCTAssertNil(parse("http://relay.example.com/invite/abc", debug: true), "plaintext is never allowed off localhost")
  }

  func testRejectsNonPublicAndMalformedIPLiterals() {
    XCTAssertNil(parse("https://192.168.1.10/invite/abc"))
    XCTAssertNil(parse("https://10.0.0.1/invite/abc"))
    XCTAssertNil(parse("https://127.0.0.1/invite/abc"))
    XCTAssertNil(parse("https://100.64.0.1/invite/abc"))
    XCTAssertNil(parse("https://2130706433/invite/abc"))
    XCTAssertNil(parse("https://0x7f.1/invite/abc"))
    XCTAssertNil(parse("https://01.2.3.4/invite/abc"))
    XCTAssertNil(parse("https://[::1]/invite/abc"))
    XCTAssertNil(parse("https://[fd00::1]/invite/abc"))
    XCTAssertNil(parse("https://[2001:db8::1]/invite/abc"))
    XCTAssertNotNil(parse("https://8.8.8.8/invite/abc"))
    XCTAssertNotNil(parse("https://[2606:4700::1111]/invite/abc"))
    XCTAssertNotNil(parse("https://[::ffff:8.8.8.8]/invite/abc"))
  }

  func testBuzzJoinHandoff() throws {
    let link = try XCTUnwrap(
      parse("buzz://join?relay=wss%3A%2F%2Frelay.example.com&code=abc&policy_receipt=r1"))
    XCTAssertEqual(link.relayURL, "wss://relay.example.com")
    XCTAssertEqual(link.code, "abc")
    XCTAssertEqual(link.policyReceipt, "r1")
    XCTAssertNil(parse("buzz://join?relay=wss://relay.example.com"))
    XCTAssertNil(parse("buzz://join?relay=https://relay.example.com&code=abc"))
    XCTAssertNil(parse("buzz://join?relay=wss://relay.example.com/path&code=abc"))
    XCTAssertNil(parse("buzz://join?relay=ws://relay.example.com&code=abc", debug: true))
    XCTAssertNil(parse("buzz://join?relay=wss://relay.example.com&code=a&code=b"))
    XCTAssertNil(parse("buzz://other?relay=wss://relay.example.com&code=abc"))
    XCTAssertEqual(
      parse("buzz://join?relay=ws://localhost:3000&code=abc", debug: true)?.relayURL, "ws://localhost:3000")
  }
}
