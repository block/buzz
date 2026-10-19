import Foundation
import Security

/// An invite the App Clip accepted on the user's behalf, waiting for the full
/// app to replay it. Only the URL is carried: the full app re-parses it with
/// the same rules it applies to a universal link, so a stale or tampered
/// record can never grant more than a scanned QR code would.
struct BuzzPendingInvite: Codable, Equatable {
  /// Invocation URL exactly as the App Clip received it.
  let url: String

  /// When the user tapped Join in the clip.
  let savedAt: Date
}

/// Storage shared between the App Clip and the full app.
///
/// App Clips cannot hold the Keychain Sharing entitlement, so the record is
/// written to two places that do cross the clip→app boundary: the writer's
/// own Keychain (iOS 15.4+ migrates an App Clip's Keychain items into the
/// full app when it is installed) and the shared App Group `UserDefaults`
/// (the fallback for older systems, and the only shared copy on the
/// simulator, where the clip and the app are two independent installs).
/// Keychain queries never pin an access group, so the full app finds the
/// migrated item wherever the system placed it. Reads prefer the Keychain and
/// `take()` clears both copies so an invite is replayed at most once.
final class BuzzPendingInviteStore {
  /// `UserDefaults` key inside the App Group suite.
  static let appGroupKey = "buzz.app-clip.pending-invite.v1"
  private static let keychainAccount = "pending-invite-v1"
  private static let keychainLabel = "Buzz App Clip pending invite"

  private let keychainService: String
  private let defaults: UserDefaults?

  /// - Parameters:
  ///   - appGroupIdentifier: App Group suite; nil disables the fallback copy.
  ///   - keychainService: Overridable so tests isolate their items.
  ///   - userDefaults: Overridable so tests avoid the real App Group suite.
  init(
    appGroupIdentifier: String?,
    keychainService: String = "buzz.app-clip.pending-invite",
    userDefaults: UserDefaults? = nil
  ) {
    self.keychainService = keychainService
    if let userDefaults {
      defaults = userDefaults
    } else if let appGroupIdentifier, !appGroupIdentifier.isEmpty {
      defaults = UserDefaults(suiteName: appGroupIdentifier)
    } else {
      defaults = nil
    }
  }

  /// Convenience for app targets whose Info.plist carries `BuzzAppGroupIdentifier`.
  convenience init(bundle: Bundle = .main) {
    self.init(appGroupIdentifier: bundle.object(forInfoDictionaryKey: "BuzzAppGroupIdentifier") as? String)
  }

  /// Persist the invite to every available copy. Succeeds if at least one copy
  /// was written; throws only when nothing durable remains.
  func save(_ invite: BuzzPendingInvite) throws {
    let data = try JSONEncoder().encode(invite)
    var keychainError: Error?
    do {
      try writeKeychain(data)
    } catch {
      keychainError = error
    }
    var defaultsWritten = false
    if let defaults {
      defaults.set(data, forKey: Self.appGroupKey)
      defaultsWritten = defaults.data(forKey: Self.appGroupKey) == data
    }
    if let keychainError, !defaultsWritten { throw keychainError }
  }

  /// Read without clearing.
  func peek() throws -> BuzzPendingInvite? {
    if let data = try readKeychain() {
      return try JSONDecoder().decode(BuzzPendingInvite.self, from: data)
    }
    if let data = defaults?.data(forKey: Self.appGroupKey) {
      return try JSONDecoder().decode(BuzzPendingInvite.self, from: data)
    }
    return nil
  }

  /// Read and clear both copies, so the invite is replayed at most once.
  func take() throws -> BuzzPendingInvite? {
    let invite = try peek()
    clear()
    return invite
  }

  /// Remove every copy. Missing items are not errors.
  func clear() {
    SecItemDelete(baseQuery() as CFDictionary)
    defaults?.removeObject(forKey: Self.appGroupKey)
  }

  private func readKeychain() throws -> Data? {
    var query = baseQuery()
    query[kSecReturnData as String] = true
    query[kSecMatchLimit as String] = kSecMatchLimitOne
    var result: CFTypeRef?
    let status = SecItemCopyMatching(query as CFDictionary, &result)
    if status == errSecItemNotFound { return nil }
    guard status == errSecSuccess else { throw keychainError(status, operation: "read") }
    return result as? Data
  }

  private func writeKeychain(_ data: Data) throws {
    let updateStatus = SecItemUpdate(baseQuery() as CFDictionary, [kSecValueData as String: data] as CFDictionary)
    if updateStatus == errSecSuccess { return }
    guard updateStatus == errSecItemNotFound else { throw keychainError(updateStatus, operation: "update") }
    var add = baseQuery()
    add[kSecValueData as String] = data
    add[kSecAttrLabel as String] = Self.keychainLabel
    add[kSecAttrAccessible as String] = kSecAttrAccessibleAfterFirstUnlockThisDeviceOnly
    let addStatus = SecItemAdd(add as CFDictionary, nil)
    guard addStatus == errSecSuccess else { throw keychainError(addStatus, operation: "add") }
  }

  private func baseQuery() -> [String: Any] {
    [
      kSecClass as String: kSecClassGenericPassword,
      kSecAttrService as String: keychainService,
      kSecAttrAccount as String: Self.keychainAccount,
    ]
  }

  private func keychainError(_ status: OSStatus, operation: String) -> Error {
    NSError(
      domain: NSOSStatusErrorDomain,
      code: Int(status),
      userInfo: [
        NSLocalizedDescriptionKey:
          "Pending invite Keychain \(operation) failed: \(SecCopyErrorMessageString(status, nil) ?? "unknown" as CFString)"
      ]
    )
  }
}
