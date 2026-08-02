import BuzzPushKit
import Foundation
import Security

/// Keychain-backed endpoint grant storage. The opaque grant is never written to
/// UserDefaults or logs. Dart can read the closed record through the push bridge.
final class BuzzPushEndpointGrantKeychainStore: BuzzPushEndpointGrantStore {
  private static let service = "buzz.push.endpoint-grants"
  private static let account = "v1"

  private let accessGroup: String?

  init(accessGroup: String?) {
    self.accessGroup = accessGroup
  }

  func records() throws -> [BuzzPushEndpointGrantRecord] {
    var query = baseQuery()
    query[kSecReturnData as String] = true
    query[kSecMatchLimit as String] = kSecMatchLimitOne
    var result: CFTypeRef?
    let status = SecItemCopyMatching(query as CFDictionary, &result)
    if status == errSecItemNotFound { return [] }
    guard status == errSecSuccess, let data = result as? Data else {
      throw keychainError(status, operation: "read")
    }
    do {
      return try JSONDecoder().decode([BuzzPushEndpointGrantRecord].self, from: data)
    } catch {
      throw NSError(
        domain: "BuzzPushEndpointGrantStore",
        code: 1,
        userInfo: [NSLocalizedDescriptionKey: "Stored endpoint grants are invalid: \(error)"]
      )
    }
  }

  func save(_ record: BuzzPushEndpointGrantRecord) throws {
    var all = try records()
    all.removeAll {
      $0.relayOrigin == record.relayOrigin && $0.appProfile == record.appProfile
    }
    all.append(record)
    try replace(all)
  }

  func markPublished(relayOrigin: String, appProfile: String, generation: Int64) throws {
    precondition(generation > 0, "Published push lease generation must be positive")
    var all = try records()
    guard
      let index = all.firstIndex(where: {
        $0.relayOrigin == relayOrigin && $0.appProfile == appProfile
      })
    else {
      throw NSError(
        domain: "BuzzPushEndpointGrantStore",
        code: 2,
        userInfo: [NSLocalizedDescriptionKey: "Cannot mark a missing endpoint grant as published."]
      )
    }
    let current = all[index]
    guard current.publishedGeneration == nil else {
      throw NSError(
        domain: "BuzzPushEndpointGrantStore",
        code: 3,
        userInfo: [
          NSLocalizedDescriptionKey:
            "Push lease generation \(current.publishedGeneration!) is already marked as published."
        ]
      )
    }
    guard current.generation == generation else {
      throw NSError(
        domain: "BuzzPushEndpointGrantStore",
        code: 4,
        userInfo: [
          NSLocalizedDescriptionKey:
            "Published generation \(generation) does not match stored grant generation \(current.generation)."
        ]
      )
    }
    all[index] = BuzzPushEndpointGrantRecord(
      relayOrigin: current.relayOrigin,
      relayPubkey: current.relayPubkey,
      installationId: current.installationId,
      endpointGrant: current.endpointGrant,
      endpointHash: current.endpointHash,
      appProfile: current.appProfile,
      endpointEpoch: current.endpointEpoch,
      generation: current.generation,
      publishedGeneration: generation,
      expiresAt: current.expiresAt
    )
    try replace(all)
  }

  private func replace(_ records: [BuzzPushEndpointGrantRecord]) throws {
    let data = try JSONEncoder().encode(records)
    let updateStatus = SecItemUpdate(
      baseQuery() as CFDictionary,
      [kSecValueData as String: data] as CFDictionary
    )
    if updateStatus == errSecSuccess { return }
    guard updateStatus == errSecItemNotFound else {
      throw keychainError(updateStatus, operation: "update")
    }

    var add = baseQuery()
    add[kSecValueData as String] = data
    add[kSecAttrAccessible as String] = kSecAttrAccessibleAfterFirstUnlockThisDeviceOnly
    let addStatus = SecItemAdd(add as CFDictionary, nil)
    guard addStatus == errSecSuccess else {
      throw keychainError(addStatus, operation: "add")
    }
  }

  private func baseQuery() -> [String: Any] {
    var query: [String: Any] = [
      kSecClass as String: kSecClassGenericPassword,
      kSecAttrService as String: Self.service,
      kSecAttrAccount as String: Self.account,
    ]
    if let accessGroup, !accessGroup.isEmpty {
      query[kSecAttrAccessGroup as String] = accessGroup
    }
    return query
  }

  private func keychainError(_ status: OSStatus, operation: String) -> Error {
    NSError(
      domain: NSOSStatusErrorDomain,
      code: Int(status),
      userInfo: [
        NSLocalizedDescriptionKey:
          "Endpoint grant Keychain \(operation) failed: \(SecCopyErrorMessageString(status, nil) ?? "unknown" as CFString)"
      ]
    )
  }
}

extension BuzzPushEndpointGrantRecord {
  var flutterArguments: [String: Any] {
    var arguments: [String: Any] = [
      "relayOrigin": relayOrigin,
      "relayPubkey": relayPubkey,
      "installationId": installationId,
      "endpointGrant": endpointGrant,
      "endpointHash": endpointHash,
      "appProfile": appProfile,
      "endpointEpoch": endpointEpoch,
      "generation": generation,
      "expiresAt": expiresAt,
    ]
    if let publishedGeneration {
      arguments["publishedGeneration"] = publishedGeneration
    }
    return arguments
  }
}
