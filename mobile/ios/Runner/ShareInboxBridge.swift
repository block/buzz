import Flutter
import Foundation

/// Exposes the Share Extension inbox to Dart over `buzz/share_inbox`.
///
/// `takePendingShare` returns the newest staged payload (or nil) and
/// `discardShare` removes its manifest once the composer has taken ownership
/// of the copied files.
final class BuzzShareInboxBridge {
  private let appGroupIdentifier: String?
  private let queue = DispatchQueue(label: "xyz.block.buzz.share-inbox", qos: .userInitiated)

  init(appGroupIdentifier: String?) {
    self.appGroupIdentifier = appGroupIdentifier
  }

  @discardableResult
  func handle(_ call: FlutterMethodCall, result: @escaping FlutterResult) -> Bool {
    switch call.method {
    case "takePendingShare":
      queue.async { [appGroupIdentifier] in
        let payload = BuzzShareInbox.takePending(appGroupIdentifier: appGroupIdentifier)
        DispatchQueue.main.async { result(payload?.flutterArguments) }
      }
    case "discardShare":
      guard let arguments = call.arguments as? [String: Any],
        let id = arguments["id"] as? String
      else {
        result(
          FlutterError(
            code: "invalid_arguments",
            message: "discardShare expects a payload id.",
            details: nil
          )
        )
        return true
      }
      queue.async { [appGroupIdentifier] in
        BuzzShareInbox.discard(id: id, appGroupIdentifier: appGroupIdentifier)
        DispatchQueue.main.async { result(nil) }
      }
    case "syncShareTargets":
      guard let arguments = call.arguments as? [String: Any],
        let communityID = arguments["communityId"] as? String,
        let rawCommunities = arguments["communities"] as? [[String: Any]],
        let rawTargets = arguments["targets"] as? [[String: Any]]
      else {
        result(
          FlutterError(
            code: "invalid_arguments",
            message: "syncShareTargets expects communities, communityId, and targets.",
            details: nil
          )
        )
        return true
      }
      let communities = rawCommunities.compactMap { raw -> BuzzShareTargets.Community? in
        guard let id = raw["id"] as? String, let name = raw["name"] as? String else { return nil }
        return BuzzShareTargets.Community(id: id, name: name)
      }
      let targets = rawTargets.compactMap { raw -> BuzzShareTargets.Target? in
        guard let channelID = raw["channelId"] as? String,
          let label = raw["label"] as? String
        else { return nil }
        return BuzzShareTargets.Target(
          communityID: communityID,
          channelID: channelID,
          label: label,
          isDM: raw["isDm"] as? Bool ?? false,
          lastMessageAt: raw["lastMessageAt"] as? Double
        )
      }
      queue.async { [appGroupIdentifier] in
        let merged = BuzzShareTargets.merge(
          into: BuzzShareTargets.read(appGroupIdentifier: appGroupIdentifier),
          communities: communities,
          communityID: communityID,
          targets: targets
        )
        do {
          try BuzzShareTargets.write(merged, appGroupIdentifier: appGroupIdentifier)
          DispatchQueue.main.async { result(nil) }
        } catch {
          DispatchQueue.main.async {
            result(
              FlutterError(
                code: "share_targets_write_failed",
                message: error.localizedDescription,
                details: nil
              )
            )
          }
        }
      }
    default:
      return false
    }
    return true
  }
}
