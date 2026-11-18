import BuzzPushKit
import Foundation
import Security
import UserNotifications

final class BuzzOneShotCompletion {
  private let lock = NSLock()
  private var completion: (() -> Void)?

  init(_ completion: @escaping () -> Void) {
    self.completion = completion
  }

  func call() {
    lock.lock()
    let completion = completion
    self.completion = nil
    lock.unlock()
    completion?()
  }
}

enum BuzzPushNotificationResponseCoordinator {
  static func handle(
    actionIdentifier: String,
    userInfo: [AnyHashable: Any],
    onTarget: (BuzzPushNavigationTarget) -> Void,
    forwardToFlutter: (@escaping () -> Void) -> Void,
    completion: @escaping () -> Void
  ) {
    let completionGate = BuzzOneShotCompletion(completion)
    defer { completionGate.call() }

    if actionIdentifier == UNNotificationDefaultActionIdentifier,
      let target = BuzzPushNavigationTarget.decodeIfPresent(from: userInfo)
    {
      onTarget(target)
    }
    forwardToFlutter { completionGate.call() }
  }
}

enum BuzzPushKeychain {
  static let service = "buzz.push.nse.signing"

  static func replace(signingKeys: [String: String], accessGroup: String?) throws {
    var query = baseQuery(accessGroup: accessGroup)
    let deletionStatus = SecItemDelete(query as CFDictionary)
    guard deletionStatus == errSecSuccess || deletionStatus == errSecItemNotFound else {
      throw NSError(
        domain: NSOSStatusErrorDomain, code: Int(deletionStatus),
        userInfo: [
          NSLocalizedDescriptionKey: SecCopyErrorMessageString(deletionStatus, nil)
            ?? "Keychain deletion failed" as CFString
        ]
      )
    }
    for (communityID, privateKeyHex) in signingKeys {
      query[kSecAttrAccount as String] = communityID
      query[kSecValueData as String] = Data(privateKeyHex.utf8)
      query[kSecAttrAccessible as String] = kSecAttrAccessibleAfterFirstUnlock
      let status = SecItemAdd(query as CFDictionary, nil)
      guard status == errSecSuccess else {
        SecItemDelete(baseQuery(accessGroup: accessGroup) as CFDictionary)
        throw NSError(
          domain: NSOSStatusErrorDomain, code: Int(status),
          userInfo: [
            NSLocalizedDescriptionKey: SecCopyErrorMessageString(status, nil)
              ?? "Keychain write failed" as CFString
          ]
        )
      }
      query.removeValue(forKey: kSecValueData as String)
      query.removeValue(forKey: kSecAttrAccessible as String)
      query.removeValue(forKey: kSecAttrAccount as String)
    }
  }

  /// Replaces a shared account session, the organization's unless `account` names the hosted
  /// one. Device-only, like the app's own copy.
  static func replace(
    organizationSession: BuzzOrganizationSession?, accessGroup: String?,
    account: String = BuzzOrganizationSession.keychainAccount
  ) throws {
    var query: [String: Any] = [
      kSecClass as String: kSecClassGenericPassword,
      kSecAttrService as String: BuzzOrganizationSession.keychainService,
      kSecAttrAccount as String: account,
    ]
    if let accessGroup, !accessGroup.isEmpty {
      query[kSecAttrAccessGroup as String] = accessGroup
    }
    let deletionStatus = SecItemDelete(query as CFDictionary)
    guard deletionStatus == errSecSuccess || deletionStatus == errSecItemNotFound else {
      throw keychainError(deletionStatus, fallback: "Keychain deletion failed")
    }
    guard let organizationSession else { return }
    query[kSecValueData as String] = try JSONEncoder().encode(organizationSession)
    query[kSecAttrAccessible as String] = kSecAttrAccessibleAfterFirstUnlockThisDeviceOnly
    let status = SecItemAdd(query as CFDictionary, nil)
    guard status == errSecSuccess else {
      throw keychainError(status, fallback: "Keychain write failed")
    }
  }

  private static func keychainError(_ status: OSStatus, fallback: String) -> NSError {
    NSError(
      domain: NSOSStatusErrorDomain, code: Int(status),
      userInfo: [
        NSLocalizedDescriptionKey: SecCopyErrorMessageString(status, nil) ?? fallback as CFString
      ]
    )
  }

  private static func baseQuery(accessGroup: String?) -> [String: Any] {
    var query: [String: Any] = [
      kSecClass as String: kSecClassGenericPassword,
      kSecAttrService as String: service,
    ]
    if let accessGroup, !accessGroup.isEmpty {
      query[kSecAttrAccessGroup as String] = accessGroup
    }
    return query
  }
}
