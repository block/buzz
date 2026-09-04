import BuzzPushKit
import Flutter
import Foundation

final class BuzzPushSnapshotBridge {
  private let appGroupIdentifier: String?
  private let endpointGrantStore: BuzzPushEndpointGrantKeychainStore
  private let keychainAccessGroup: String?
  private let queue = DispatchQueue(
    label: "xyz.block.buzz.push-snapshot",
    qos: .utility
  )
  private lazy var store: BuzzPushPresentationCacheStore? = {
    guard let appGroupIdentifier,
      let container = FileManager.default.containerURL(
        forSecurityApplicationGroupIdentifier: appGroupIdentifier
      )
    else { return nil }
    return BuzzPushPresentationCacheStore(containerURL: container)
  }()

  init(
    appGroupIdentifier: String?,
    endpointGrantStore: BuzzPushEndpointGrantKeychainStore,
    keychainAccessGroup: String?
  ) {
    self.appGroupIdentifier = appGroupIdentifier
    self.endpointGrantStore = endpointGrantStore
    self.keychainAccessGroup = keychainAccessGroup
  }

  @discardableResult
  func handle(_ call: FlutterMethodCall, result: @escaping FlutterResult) -> Bool {
    guard call.method == "syncPushSnapshot",
      let arguments = call.arguments as? [String: Any],
      let section = arguments["section"] as? String
    else {
      return false
    }
    switch section {
    case "communities": syncCommunities(arguments, result: result)
    case "profiles": cacheProfiles(arguments, result: result)
    case "channels": cacheChannels(arguments, result: result)
    case "avatar": cacheAvatar(arguments, result: result)
    default: return false
    }
    return true
  }

  private func syncCommunities(_ arguments: [String: Any], result: @escaping FlutterResult) {
    guard let communities = arguments["communities"] as? [[String: Any]],
      let signingKeys = arguments["signingKeys"] as? [String: String],
      communities.count <= BuzzPushPresentationCacheStore.maximumCommunities
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
        guard let self, let store else {
          Self.complete(result, value: nil)
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
        try store.replaceCommunities(decoded)
        try BuzzPushKeychain.replace(
          signingKeys: signingKeys,
          accessGroup: keychainAccessGroup
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
      $0.appProfile == BuzzDevPushEnrollmentDriver.appProfile
        && BuzzPushPresentationCacheStore.canonicalRelayOrigin($0.relayOrigin) == origin
    }.max {
      $0.generation < $1.generation
    }?.relayMetadataPubkey
  }

  private func cacheProfiles(_ rawArguments: Any?, result: @escaping FlutterResult) {
    guard let arguments = rawArguments as? [String: Any],
      let communityID = arguments["communityId"] as? String,
      let rawEvents = arguments["events"] as? [[String: Any]],
      rawEvents.count <= BuzzPushPresentationCacheStore.maximumProfiles
    else {
      result(
        FlutterError(
          code: "invalid_arguments",
          message: "Expected communityId and profile events.",
          details: nil
        )
      )
      return
    }
    queue.async { [weak self] in
      do {
        guard let self, let community = community(id: communityID) else {
          Self.complete(result, value: nil)
          return
        }
        let events = try decodeEvents(rawEvents)
        try store?.updateProfiles(
          communityID: communityID,
          relayOrigin: community.relayUrl,
          updates: events.map { BuzzPushProfileCacheUpdate(event: $0) }
        )
        Self.complete(result, value: nil)
      } catch {
        Self.complete(
          result,
          value: FlutterError(
            code: "profile_cache_failed",
            message: "Unable to cache push sender profiles.",
            details: error.localizedDescription
          )
        )
      }
    }
  }

  private func cacheChannels(_ rawArguments: Any?, result: @escaping FlutterResult) {
    guard let arguments = rawArguments as? [String: Any],
      let communityID = arguments["communityId"] as? String,
      let rawMetadataEvents = arguments["metadataEvents"] as? [[String: Any]],
      let rawMembershipEvents = arguments["membershipEvents"] as? [[String: Any]],
      rawMetadataEvents.count <= BuzzPushPresentationCacheStore.maximumChannels,
      rawMembershipEvents.count <= BuzzPushPresentationCacheStore.maximumChannels
    else {
      result(
        FlutterError(
          code: "invalid_arguments",
          message: "Expected communityId, channel metadata, and membership events.",
          details: nil
        )
      )
      return
    }
    queue.async { [weak self] in
      do {
        guard let self, let community = community(id: communityID),
          let relayMetadataPubkey = community.relayMetadataPubkey
        else {
          Self.complete(result, value: nil)
          return
        }
        try store?.updateChannels(
          communityID: communityID,
          relayOrigin: community.relayUrl,
          relayMetadataPubkey: relayMetadataPubkey,
          metadataEvents: try decodeEvents(rawMetadataEvents),
          membershipEvents: try decodeEvents(rawMembershipEvents)
        )
        Self.complete(result, value: nil)
      } catch {
        Self.complete(
          result,
          value: FlutterError(
            code: "channel_cache_failed",
            message: "Unable to cache push channel metadata.",
            details: error.localizedDescription
          )
        )
      }
    }
  }

  private func cacheAvatar(_ rawArguments: Any?, result: @escaping FlutterResult) {
    guard let arguments = rawArguments as? [String: Any],
      let communityID = arguments["communityId"] as? String,
      let sourceURL = arguments["sourceUrl"] as? String,
      let avatar = arguments["png"] as? FlutterStandardTypedData
    else {
      result(
        FlutterError(
          code: "invalid_arguments",
          message: "Expected an avatar source and PNG thumbnail.",
          details: nil
        )
      )
      return
    }
    let avatarData = avatar.data
    queue.async { [weak self] in
      do {
        guard let self, let community = community(id: communityID) else {
          Self.complete(result, value: false)
          return
        }
        let updated =
          try store?.updateAvatar(
            communityID: communityID,
            relayOrigin: community.relayUrl,
            sourceURL: sourceURL,
            avatarPNG: avatarData
          ) ?? false
        Self.complete(result, value: updated)
      } catch {
        Self.complete(
          result,
          value: FlutterError(
            code: "avatar_cache_failed",
            message: "Unable to cache a push sender avatar.",
            details: error.localizedDescription
          )
        )
      }
    }
  }

  private func community(id: String) -> PushLeaseCommunity? {
    guard let appGroupIdentifier,
      let container = FileManager.default.containerURL(
        forSecurityApplicationGroupIdentifier: appGroupIdentifier
      ),
      let data = try? Data(contentsOf: container.appendingPathComponent(BuzzPushPresentationCacheStore.fileName)),
      let snapshot = try? JSONDecoder().decode(BuzzPushPresentationCacheSnapshot.self, from: data)
    else { return nil }
    return snapshot.communities.first { $0.id == id }
  }

  private func decodeEvents(_ rawEvents: [[String: Any]]) throws -> [VerifiedNostrEvent] {
    let data = try JSONSerialization.data(withJSONObject: rawEvents)
    return try JSONDecoder().decode([VerifiedNostrEvent].self, from: data)
  }

  private static func complete(_ result: @escaping FlutterResult, value: Any?) {
    DispatchQueue.main.async {
      result(value)
    }
  }
}
