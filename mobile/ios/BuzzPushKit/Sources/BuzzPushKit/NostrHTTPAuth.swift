import CryptoKit
import Foundation
import P256K

public enum NostrHTTPAuthError: Error, Equatable {
    case invalidHex
    case signingFailed
}

public struct VerifiedNostrEvent: Codable, Equatable, Sendable {
    public let id: String
    public let pubkey: String
    public let createdAt: Int
    public let kind: Int
    public let tags: [[String]]
    public let content: String
    public let sig: String

    enum CodingKeys: String, CodingKey {
        case id, pubkey, kind, tags, content, sig
        case createdAt = "created_at"
    }

    public init(
        id: String, pubkey: String, createdAt: Int, kind: Int,
        tags: [[String]], content: String, sig: String
    ) {
        self.id = id
        self.pubkey = pubkey
        self.createdAt = createdAt
        self.kind = kind
        self.tags = tags
        self.content = content
        self.sig = sig
    }

    public func hasValidIDAndSignature() -> Bool {
        guard let idBytes = Self.hexBytes(id), idBytes.count == 32,
              let pubkeyBytes = Self.hexBytes(pubkey), pubkeyBytes.count == 32,
              let signatureBytes = Self.hexBytes(sig), signatureBytes.count == 64,
              let serialized = try? Self.canonicalSerialization(
                pubkey: pubkey.lowercased(), createdAt: createdAt, kind: kind,
                tags: tags, content: content
              )
        else { return false }
        let digest = Array(SHA256.hash(data: serialized))
        guard digest == idBytes,
              let signature = try? P256K.Schnorr.SchnorrSignature(
                dataRepresentation: Data(signatureBytes)
              )
        else { return false }
        var message = digest
        let key = P256K.Schnorr.XonlyKey(dataRepresentation: pubkeyBytes)
        return key.isValid(signature, for: &message)
    }

    static func canonicalSerialization(
        pubkey: String, createdAt: Int, kind: Int, tags: [[String]], content: String
    ) throws -> Data {
        try JSONSerialization.data(
            withJSONObject: [0, pubkey, createdAt, kind, tags, content],
            options: [.withoutEscapingSlashes]
        )
    }

    static func hexBytes(_ value: String) -> [UInt8]? {
        guard value.count.isMultiple(of: 2) else { return nil }
        var result: [UInt8] = []
        result.reserveCapacity(value.count / 2)
        var index = value.startIndex
        while index < value.endIndex {
            let end = value.index(index, offsetBy: 2)
            guard let byte = UInt8(value[index..<end], radix: 16) else { return nil }
            result.append(byte)
            index = end
        }
        return result
    }

    static func hex(_ bytes: some Sequence<UInt8>) -> String {
        bytes.map { String(format: "%02x", $0) }.joined()
    }
}

public enum NostrHTTPAuth {
    public static func authorizationHeader(
        url: URL,
        method: String,
        body: Data,
        privateKeyHex: String,
        createdAt: Int = Int(Date().timeIntervalSince1970),
        auxiliaryRandomness: [UInt8]? = nil
    ) throws -> String {
        guard let privateKeyBytes = VerifiedNostrEvent.hexBytes(privateKeyHex),
              privateKeyBytes.count == 32
        else { throw NostrHTTPAuthError.invalidHex }
        do {
            let privateKey = try P256K.Schnorr.PrivateKey(
                dataRepresentation: privateKeyBytes
            )
            let pubkey = VerifiedNostrEvent.hex(privateKey.xonly.bytes)
            let payload = VerifiedNostrEvent.hex(SHA256.hash(data: body))
            let tags = [
                ["u", url.absoluteString],
                ["method", method.uppercased()],
                ["payload", payload],
            ]
            let serialized = try VerifiedNostrEvent.canonicalSerialization(
                pubkey: pubkey, createdAt: createdAt, kind: 27235,
                tags: tags, content: ""
            )
            let digest = Array(SHA256.hash(data: serialized))
            var message = digest
            let signature: P256K.Schnorr.SchnorrSignature
            if var randomness = auxiliaryRandomness {
                guard randomness.count == 32 else { throw NostrHTTPAuthError.signingFailed }
                signature = try privateKey.signature(
                    message: &message, auxiliaryRand: &randomness
                )
            } else {
                signature = try privateKey.signature(
                    message: &message, auxiliaryRand: nil
                )
            }
            let event = VerifiedNostrEvent(
                id: VerifiedNostrEvent.hex(digest),
                pubkey: pubkey,
                createdAt: createdAt,
                kind: 27235,
                tags: tags,
                content: "",
                sig: VerifiedNostrEvent.hex(signature.dataRepresentation)
            )
            let encoder = JSONEncoder()
            encoder.outputFormatting = [.withoutEscapingSlashes]
            return "Nostr " + (try encoder.encode(event)).base64EncodedString()
        } catch let error as NostrHTTPAuthError {
            throw error
        } catch {
            throw NostrHTTPAuthError.signingFailed
        }
    }
}
