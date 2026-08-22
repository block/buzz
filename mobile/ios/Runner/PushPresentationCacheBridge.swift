#if BUZZ_PUSH_ENABLED
  import BuzzPushKit
  import Flutter
  import Foundation

final class BuzzPushPresentationCacheBridge {
  private let appGroupIdentifier: String?
  private let queue = DispatchQueue(
    label: "xyz.block.buzz.push-presentation-cache",
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

  init(appGroupIdentifier: String?) {
    self.appGroupIdentifier = appGroupIdentifier
  }

  @discardableResult
  func handle(_ call: FlutterMethodCall, result: @escaping FlutterResult) -> Bool {
    switch call.method {
    case "cachePresentationProfiles":
      cacheProfiles(call.arguments, result: result)
    case "cachePresentationChannels":
      cacheChannels(call.arguments, result: result)
    case "cachePresentationAvatar":
      cacheAvatar(call.arguments, result: result)
    default:
      return false
    }
    return true
  }

  func retainCommunities(_ communityIDs: Set<String>) {
    queue.async { [weak self] in
      try? self?.store?.retainCommunities(communityIDs)
    }
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
        let updated = try store?.updateAvatar(
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
      let data = try? Data(
        contentsOf: container.appendingPathComponent("push-communities.json")
      ),
      let snapshot = try? JSONDecoder().decode(PushLeaseSnapshot.self, from: data)
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
#endif
