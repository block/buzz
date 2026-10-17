import CSecp256k1
import Foundation
import Security

/// The BIP-340 operations used by Nostr. C objects never escape this adapter.
enum NostrSchnorr {
  enum Failure: Error { case invalidInput, randomSource, operation }

  static func publicKey(secret: [UInt8]) throws -> [UInt8] {
    guard secret.count == 32 else { throw Failure.invalidInput }
    return try withContext { context in
      var pair = secp256k1_keypair()
      guard secp256k1_keypair_create(context, &pair, secret) == 1 else {
        throw Failure.invalidInput
      }
      defer {
        _ = withUnsafeMutableBytes(of: &pair) { $0.initializeMemory(as: UInt8.self, repeating: 0) }
      }
      var key = secp256k1_xonly_pubkey()
      var output = [UInt8](repeating: 0, count: 32)
      guard secp256k1_keypair_xonly_pub(context, &key, nil, &pair) == 1,
        secp256k1_xonly_pubkey_serialize(context, &output, &key) == 1
      else { throw Failure.operation }
      return output
    }
  }

  static func sign(message: [UInt8], secret: [UInt8], auxiliaryRandomness: [UInt8]? = nil) throws
    -> [UInt8]
  {
    guard message.count == 32, secret.count == 32 else { throw Failure.invalidInput }
    let randomness = try auxiliaryRandomness ?? randomBytes()
    guard randomness.count == 32 else { throw Failure.invalidInput }
    return try withContext { context in
      var pair = secp256k1_keypair()
      guard secp256k1_keypair_create(context, &pair, secret) == 1 else {
        throw Failure.invalidInput
      }
      defer {
        _ = withUnsafeMutableBytes(of: &pair) { $0.initializeMemory(as: UInt8.self, repeating: 0) }
      }
      var signature = [UInt8](repeating: 0, count: 64)
      var key = secp256k1_xonly_pubkey()
      guard secp256k1_schnorrsig_sign32(context, &signature, message, &pair, randomness) == 1,
        secp256k1_keypair_xonly_pub(context, &key, nil, &pair) == 1,
        secp256k1_schnorrsig_verify(context, signature, message, message.count, &key) == 1
      else { throw Failure.operation }
      return signature
    }
  }

  static func verify(signature: [UInt8], message: [UInt8], publicKey: [UInt8]) -> Bool {
    guard signature.count == 64, message.count == 32, publicKey.count == 32 else { return false }
    var key = secp256k1_xonly_pubkey()
    return secp256k1_xonly_pubkey_parse(secp256k1_context_static, &key, publicKey) == 1
      && secp256k1_schnorrsig_verify(
        secp256k1_context_static, signature, message, message.count, &key) == 1
  }

  private static func randomBytes() throws -> [UInt8] {
    var bytes = [UInt8](repeating: 0, count: 32)
    guard SecRandomCopyBytes(kSecRandomDefault, bytes.count, &bytes) == errSecSuccess else {
      throw Failure.randomSource
    }
    return bytes
  }

  private static func withContext<T>(_ body: (OpaquePointer) throws -> T) throws -> T {
    guard let context = secp256k1_context_create(UInt32(SECP256K1_CONTEXT_NONE)) else {
      throw Failure.operation
    }
    defer { secp256k1_context_destroy(context) }
    let seed = try randomBytes()
    guard secp256k1_context_randomize(context, seed) == 1 else { throw Failure.operation }
    return try body(context)
  }
}
