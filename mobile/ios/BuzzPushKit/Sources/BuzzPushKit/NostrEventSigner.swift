import CryptoKit
import Foundation

/// Signs arbitrary Nostr events for extensions that publish on their own.
public enum NostrEventSigner {
  public static func sign(
    kind: Int, content: String, tags: [[String]], privateKeyHex: String,
    createdAt: Int = Int(Date().timeIntervalSince1970)
  ) throws -> VerifiedNostrEvent {
    guard let secret = VerifiedNostrEvent.hexBytes(privateKeyHex), secret.count == 32 else {
      throw NostrHTTPAuthError.invalidHex
    }
    let pubkey = VerifiedNostrEvent.hex(try NostrSchnorr.publicKey(secret: secret))
    let serialized = try VerifiedNostrEvent.canonicalSerialization(
      pubkey: pubkey, createdAt: createdAt, kind: kind, tags: tags, content: content)
    let digest = Array(SHA256.hash(data: serialized))
    let signature = try NostrSchnorr.sign(message: digest, secret: secret)
    return VerifiedNostrEvent(
      id: VerifiedNostrEvent.hex(digest), pubkey: pubkey, createdAt: createdAt, kind: kind,
      tags: tags, content: content, sig: VerifiedNostrEvent.hex(signature))
  }

  /// The x-only public key for `privateKeyHex`.
  public static func publicKey(privateKeyHex: String) -> String? {
    guard let secret = VerifiedNostrEvent.hexBytes(privateKeyHex), secret.count == 32,
      let key = try? NostrSchnorr.publicKey(secret: secret)
    else { return nil }
    return VerifiedNostrEvent.hex(key)
  }
}
