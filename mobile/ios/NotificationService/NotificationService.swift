import Foundation
import UserNotifications

final class NotificationService: UNNotificationServiceExtension {
  private var contentHandler: ((UNNotificationContent) -> Void)?
  private var bestAttemptContent: UNMutableNotificationContent?
  private var resolver: BuzzPushNotificationResolving = BuzzPushNotificationResolver()

  override func didReceive(
    _ request: UNNotificationRequest,
    withContentHandler contentHandler: @escaping (UNNotificationContent) -> Void
  ) {
    self.contentHandler = contentHandler
    guard let content = request.content.mutableCopy() as? UNMutableNotificationContent else {
      contentHandler(request.content)
      return
    }
    bestAttemptContent = content

    resolver.resolve { [weak self] resolution in
      guard let self else { return }
      if let resolution {
        content.title = resolution.title
        content.body = resolution.body
        if let subtitle = resolution.subtitle {
          content.subtitle = subtitle
        }
        if let threadIdentifier = resolution.threadIdentifier {
          content.threadIdentifier = threadIdentifier
        }
      }
      self.finish(content)
    }
  }

  override func serviceExtensionTimeWillExpire() {
    if let bestAttemptContent {
      finish(bestAttemptContent)
    }
  }

  private func finish(_ content: UNNotificationContent) {
    guard let contentHandler else { return }
    self.contentHandler = nil
    contentHandler(content)
  }
}

struct BuzzPushResolution: Decodable {
  let title: String
  let body: String
  let subtitle: String?
  let threadIdentifier: String?
}

protocol BuzzPushNotificationResolving {
  func resolve(completion: @escaping (BuzzPushResolution?) -> Void)
}

final class BuzzPushNotificationResolver: BuzzPushNotificationResolving {
  private let snapshotFile = "push-communities.json"
  private let session: URLSession
  private let appGroupIdentifier: String?

  init(
    session: URLSession = .shared,
    appGroupIdentifier: String? = Bundle.main.object(forInfoDictionaryKey: "BuzzAppGroupIdentifier")
      as? String
  ) {
    self.session = session
    self.appGroupIdentifier = appGroupIdentifier
  }

  func resolve(completion: @escaping (BuzzPushResolution?) -> Void) {
    guard let community = loadCommunities().first(where: { $0.pubkey?.isEmpty == false }) else {
      completion(nil)
      return
    }

    // The NIP-PL APNs payload intentionally carries no relay or event id. The
    // service extension therefore performs a bounded catch-up against locally
    // configured origins and only replaces the fixed placeholder if authoritative
    // relay data is available before the NSE deadline.
    let filters: [[String: Any]] = [
      [
        "kinds": [9, 40002, 45001, 45003],
        "#p": [community.pubkey!],
        "limit": 10,
      ]
    ]
    guard let body = try? JSONSerialization.data(withJSONObject: filters) else {
      completion(nil)
      return
    }
    let url = URL(string: "/query", relativeTo: community.relayURL)!
    var request = URLRequest(url: url)
    request.httpMethod = "POST"
    request.setValue("application/json", forHTTPHeaderField: "Content-Type")
    request.httpBody = body

    session.dataTask(with: request) { data, response, _ in
      guard let httpResponse = response as? HTTPURLResponse,
        (200..<300).contains(httpResponse.statusCode),
        let data,
        let events = try? JSONSerialization.jsonObject(with: data) as? [[String: Any]],
        !events.isEmpty
      else {
        completion(nil)
        return
      }
      let resolution = Self.decodeResolution(
        events: events,
        community: community
      )
      completion(resolution)
    }.resume()
  }

  private static func decodeResolution(events: [[String: Any]], community: BuzzPushCommunity)
    -> BuzzPushResolution?
  {
    guard let myPubkey = community.pubkey?.lowercased() else { return nil }
    let candidates = events.compactMap(BuzzPushEvent.init(json:)).filter { event in
      event.pubkey.lowercased() != myPubkey && [9, 40002, 45001, 45003].contains(event.kind)
    }
    guard
      let event = candidates.sorted(by: { left, right in
        if left.createdAt != right.createdAt { return left.createdAt > right.createdAt }
        return left.id < right.id
      }).first
    else { return nil }
    let body = previewBody(event.content)
    guard !body.isEmpty else { return nil }
    return BuzzPushResolution(
      title: shortPubkey(event.pubkey),
      body: body,
      subtitle: community.name,
      threadIdentifier: event.channelId ?? community.id
    )
  }

  private static func previewBody(_ content: String) -> String {
    var result = content.replacingOccurrences(
      of: #"```[\s\S]*?```"#,
      with: "[code]",
      options: .regularExpression
    )
    result = result.replacingOccurrences(of: #"`([^`]*)`"#, with: "$1", options: .regularExpression)
    result = result.replacingOccurrences(
      of: #"!\[([^\]]*)\]\([^)]*\)"#, with: "$1", options: .regularExpression)
    result = result.replacingOccurrences(
      of: #"\[([^\]]+)\]\([^)]*\)"#, with: "$1", options: .regularExpression)
    result = result.replacingOccurrences(
      of: #"https?://\S+"#, with: "[link]", options: .regularExpression)
    result = result.replacingOccurrences(of: #"\s+"#, with: " ", options: .regularExpression)
      .trimmingCharacters(in: .whitespacesAndNewlines)
    guard result.count > 180 else { return result }
    return String(result.prefix(177)).trimmingCharacters(in: .whitespacesAndNewlines) + "…"
  }

  private static func shortPubkey(_ pubkey: String) -> String {
    guard pubkey.count > 8 else { return pubkey }
    return String(pubkey.prefix(8)) + "…"
  }

  private func loadCommunities() -> [BuzzPushCommunity] {
    guard let appGroupIdentifier,
      let container = FileManager.default.containerURL(
        forSecurityApplicationGroupIdentifier: appGroupIdentifier)
    else { return [] }
    let url = container.appendingPathComponent(snapshotFile)
    guard let data = try? Data(contentsOf: url),
      let decoded = try? JSONDecoder().decode(BuzzPushSnapshot.self, from: data)
    else { return [] }
    return decoded.communities
  }
}

struct BuzzPushEvent {
  let id: String
  let pubkey: String
  let createdAt: Int
  let kind: Int
  let tags: [[String]]
  let content: String

  init?(json: [String: Any]) {
    guard let id = json["id"] as? String,
      let pubkey = json["pubkey"] as? String,
      let createdAt = json["created_at"] as? Int,
      let kind = json["kind"] as? Int,
      let tags = json["tags"] as? [[String]],
      let content = json["content"] as? String
    else { return nil }
    self.id = id
    self.pubkey = pubkey
    self.createdAt = createdAt
    self.kind = kind
    self.tags = tags
    self.content = content
  }

  var channelId: String? {
    tags.first { $0.count >= 2 && $0[0] == "h" }?[1]
  }
}

struct BuzzPushSnapshot: Decodable {
  let communities: [BuzzPushCommunity]
}

struct BuzzPushCommunity: Decodable {
  let id: String
  let name: String
  let relayUrl: String
  let pubkey: String?

  var relayURL: URL {
    URL(string: relayUrl) ?? URL(string: "http://127.0.0.1")!
  }
}
