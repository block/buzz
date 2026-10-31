import CryptoKit
import Foundation

/// RFC 9180 base-mode sealing only. Enrollment, key ownership, durable envelope
/// storage and upload are enforced by the hosted coordinator, never by a link.
/// The caller must pass AAD built from the authenticated enrollment contract.
@available(iOS 17.0, macOS 14.0, *)
enum HostedHpkeBackup {
  enum Failure: Error { case invalidInput }

  static func seal(recipient: Data, plaintext: Data, aad: Data) throws -> (enc: Data, ciphertext: Data) {
    guard recipient.count == 65, recipient.first == 4,
      plaintext.count == 32, !aad.isEmpty
    else { throw Failure.invalidInput }
    let publicKey = try P256.KeyAgreement.PublicKey(x963Representation: recipient)
    var sender = try HPKE.Sender(
      recipientKey: publicKey,
      ciphersuite: .P256_SHA256_AES_GCM_256,
      info: Data("buzz/nsec-backup/v1".utf8)
    )
    let ciphertext = try sender.seal(plaintext, authenticating: aad)
    guard sender.encapsulatedKey.count == 65, ciphertext.count == 48
    else { throw Failure.invalidInput }
    return (sender.encapsulatedKey, ciphertext)
  }
}
