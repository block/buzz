import BuzzPushKit
import Flutter
import Foundation
import Intents
import UserNotifications

final class BuzzPushSnapshotBridge {
  private let containerURL: () -> URL?
  private let endpointGrantStore: BuzzPushEndpointGrantKeychainStore
  private let interactionDeletionDeadline: BuzzInteractionDeletionDeadline
  private let keychainAccessGroup: String?
  private let queue = DispatchQueue(
    label: "xyz.block.buzz.push-snapshot",
    qos: .utility
  )
  private lazy var store: BuzzPushPresentationCacheStore? = {
    guard let container = containerURL() else { return nil }
    return BuzzPushPresentationCacheStore(containerURL: container)
  }()
  private let ageRestrictionSession = BuzzAgeRestrictionSession()

  init(
    appGroupIdentifier: String?,
    endpointGrantStore: BuzzPushEndpointGrantKeychainStore,
    keychainAccessGroup: String?,
    containerURL: (() -> URL?)? = nil,
    interactionDeletionDeadline: BuzzInteractionDeletionDeadline =
      BuzzInteractionDeletionDeadline(
        timeout: 5,
        deleteAllInteractions: { completion in
          INInteraction.deleteAll(completion: completion)
        },
        scheduleTimeout: { delay, action in
          DispatchQueue.global(qos: .utility).asyncAfter(
            deadline: .now() + delay,
            execute: action
          )
        }
      )
  ) {
    self.containerURL = containerURL ?? {
      guard let appGroupIdentifier else { return nil }
      return FileManager.default.containerURL(
        forSecurityApplicationGroupIdentifier: appGroupIdentifier)
    }
    self.endpointGrantStore = endpointGrantStore
    self.keychainAccessGroup = keychainAccessGroup
    self.interactionDeletionDeadline = interactionDeletionDeadline
  }

  @discardableResult
  func handle(_ call: FlutterMethodCall, result: @escaping FlutterResult) -> Bool {
    if call.method == "restoreAgeRestrictedNotifications" {
      queue.async { [self] in
        ageRestrictionSession.release()
        BuzzInteractionCleanupRetry(
          containerURL: containerURL(), deletion: interactionDeletionDeadline
        ).retryPending { error in
          Self.complete(result, value: error.map {
            FlutterError(code: "interaction_cleanup_retry_failed",
              message: "Unable to finish pending interaction cleanup.",
              details: $0.localizedDescription)
          })
        }
      }
      return true
    }
    if call.method == "purgeAgeRestrictedNotifications" {
      purgeAgeRestrictedNotifications(result: result)
      return true
    }
    let strictAgeGateWrite = call.method == "syncAgeGatePushSnapshot"
    guard strictAgeGateWrite || call.method == "syncPushSnapshot",
      let arguments = call.arguments as? [String: Any],
      let section = arguments["section"] as? String
    else {
      return false
    }
    switch section {
    case "communities":
      syncCommunities(
        arguments,
        requiresStore: strictAgeGateWrite,
        result: result
      )
    default: return false
    }
    return true
  }

  private func purgeAgeRestrictedNotifications(result: @escaping FlutterResult) {
    queue.async { [weak self] in
      do {
        guard let self, let container = containerURL() else {
          throw NSError(
            domain: "BuzzPushSnapshotBridge",
            code: 1,
            userInfo: [
              NSLocalizedDescriptionKey: "The age-restriction session container is unavailable."
            ]
          )
        }
        try ageRestrictionSession.restrict(containerURL: container)
        // Preserve snapshots and credentials. Restriction authority expires
        // with this process even when cleanup or a later storage read fails.
        let center = UNUserNotificationCenter.current()
        center.removeAllDeliveredNotifications()
        center.removeAllPendingNotificationRequests()
        BuzzInteractionCleanupRetry(
          containerURL: container, deletion: interactionDeletionDeadline
        ).request { error in
          Self.complete(
            result,
            value: error.map {
              FlutterError(
                code: "age_restriction_purge_failed",
                message: "Unable to suppress and purge restricted notifications.",
                details: $0.localizedDescription
              )
            }
          )
        }
      } catch {
        Self.complete(
          result,
          value: FlutterError(
            code: "age_restriction_purge_failed",
            message: "Unable to suppress and purge restricted notifications.",
            details: error.localizedDescription
          )
        )
      }
    }
  }

  private func syncCommunities(
    _ arguments: [String: Any],
    requiresStore: Bool,
    result: @escaping FlutterResult
  ) {
    guard let communities = arguments["communities"] as? [[String: Any]],
      let signingKeys = arguments["signingKeys"] as? [String: String],
      communities.count <= BuzzPushPresentationCacheStore.maximumCommunities,
      !requiresStore || arguments["settleFence"] is Bool
    else {
      result(
        FlutterError(
          code: "invalid_arguments",
          message: "Expected bounded communities and signing keys.",
          details: nil
        )
      )
      return
    }
    queue.async { [weak self] in
      do {
        guard let self else {
          Self.complete(
            result,
            value: requiresStore
              ? FlutterError(
                code: "snapshot_sync_unavailable",
                message: "The push snapshot bridge is unavailable.",
                details: nil
              )
              : nil
          )
          return
        }
        guard let store else {
          Self.complete(
            result,
            value: requiresStore
              ? FlutterError(
                code: "snapshot_sync_unavailable",
                message: "The push snapshot store is unavailable.",
                details: nil
              )
              : nil
          )
          return
        }
        // Relay-metadata enrichment is optional presentation state. A damaged
        // grant cache must not block the core NSE snapshot and key update.
        let grants = (try? endpointGrantStore.records()) ?? []
        let enriched = communities.map { community -> [String: Any] in
          var community = community
          guard let relayURL = community["relayUrl"] as? String,
            let relayMetadataPubkey = Self.relayMetadataPubkey(
              relayURL: relayURL,
              grants: grants
            )
          else { return community }
          community["relayMetadataPubkey"] = relayMetadataPubkey
          return community
        }
        let data = try JSONSerialization.data(withJSONObject: enriched, options: [.sortedKeys])
        let decoded = try JSONDecoder().decode([PushLeaseCommunity].self, from: data)
        // Ordinary snapshot maintenance never changes age access.
        try store.replaceCommunities(decoded)
        try BuzzPushKeychain.replace(
          signingKeys: signingKeys,
          accessGroup: self.keychainAccessGroup
        )
        Self.complete(result, value: nil)
      } catch {
        Self.complete(
          result,
          value: FlutterError(
            code: "snapshot_sync_failed",
            message: "Unable to sync push community state.",
            details: error.localizedDescription
          )
        )
      }
    }
  }

  static func relayMetadataPubkey(
    relayURL: String,
    grants: [BuzzPushEndpointGrantRecord]
  ) -> String? {
    guard let origin = BuzzPushPresentationCacheStore.canonicalRelayOrigin(relayURL) else {
      return nil
    }
    return grants.filter {
      BuzzPushPresentationCacheStore.canonicalRelayOrigin($0.relayOrigin) == origin
    }.max {
      $0.generation < $1.generation
    }?.relayMetadataPubkey
  }

  private static func complete(_ result: @escaping FlutterResult, value: Any?) {
    DispatchQueue.main.async {
      result(value)
    }
  }
}
