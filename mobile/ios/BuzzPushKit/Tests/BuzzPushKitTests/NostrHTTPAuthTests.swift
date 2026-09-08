import CryptoKit
import Foundation
import XCTest

@testable import BuzzPushKit

final class NostrHTTPAuthTests: XCTestCase {
    private let privateKey = String(repeating: "0", count: 63) + "1"
    private let expectedPubkey =
        "79be667ef9dcbbac55a06295ce870b07029bfcdb2dce28d959f2815b16f81798"

    func testAuthorizationHeaderConstructsValidNIP98Event() throws {
        let body = Data("[{\"kinds\":[9]}]".utf8)
        let url = URL(string: "https://relay.example/query")!
        let header = try NostrHTTPAuth.authorizationHeader(
            url: url,
            method: "post",
            body: body,
            privateKeyHex: privateKey,
            createdAt: 1_700_000_000,
            auxiliaryRandomness: [UInt8](repeating: 0, count: 32)
        )

        XCTAssertTrue(header.hasPrefix("Nostr "))
        let encoded = try XCTUnwrap(Data(base64Encoded: String(header.dropFirst(6))))
        let event = try JSONDecoder().decode(VerifiedNostrEvent.self, from: encoded)
        XCTAssertEqual(event.pubkey, expectedPubkey)
        XCTAssertEqual(event.createdAt, 1_700_000_000)
        XCTAssertEqual(event.kind, 27235)
        XCTAssertEqual(event.content, "")
        XCTAssertEqual(event.tags, [
            ["u", "https://relay.example/query"],
            ["method", "POST"],
            ["payload", SHA256.hash(data: body).map { String(format: "%02x", $0) }.joined()],
        ])
        XCTAssertTrue(event.hasValidIDAndSignature())
    }

    func testEventVerificationRejectsChangedIDSignatureAndContent() throws {
        let event = try makeEvent()
        XCTAssertTrue(event.hasValidIDAndSignature())
        XCTAssertFalse(copy(event, id: String(repeating: "0", count: 64)).hasValidIDAndSignature())
        XCTAssertFalse(copy(event, sig: String(repeating: "0", count: 128)).hasValidIDAndSignature())
        XCTAssertFalse(copy(event, content: "tampered").hasValidIDAndSignature())
    }

    private func makeEvent() throws -> VerifiedNostrEvent {
        let header = try NostrHTTPAuth.authorizationHeader(
            url: URL(string: "https://relay.example/query")!,
            method: "POST",
            body: Data(),
            privateKeyHex: privateKey,
            createdAt: 1_700_000_000,
            auxiliaryRandomness: [UInt8](repeating: 0, count: 32)
        )
        let data = try XCTUnwrap(Data(base64Encoded: String(header.dropFirst(6))))
        return try JSONDecoder().decode(VerifiedNostrEvent.self, from: data)
    }

    private func copy(
        _ event: VerifiedNostrEvent,
        id: String? = nil,
        content: String? = nil,
        sig: String? = nil
    ) -> VerifiedNostrEvent {
        VerifiedNostrEvent(
            id: id ?? event.id,
            pubkey: event.pubkey,
            createdAt: event.createdAt,
            kind: event.kind,
            tags: event.tags,
            content: content ?? event.content,
            sig: sig ?? event.sig
        )
    }
}
