import BuzzPushKit
import Foundation
import Security
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
  private let session: URLSession
  private let appGroupIdentifier: String?
  private let keychainAccessGroup: String?
  private let defaults: UserDefaults?

  init(
    session: URLSession = .shared,
    appGroupIdentifier: String? = Bundle.main.object(forInfoDictionaryKey: "BuzzAppGroupIdentifier") as? String,
    keychainAccessGroup: String? = Bundle.main.object(forInfoDictionaryKey: "BuzzKeychainAccessGroup") as? String
  ) {
    self.session = session
    self.appGroupIdentifier = appGroupIdentifier
    self.keychainAccessGroup = keychainAccessGroup
    defaults = appGroupIdentifier.flatMap(UserDefaults.init(suiteName:))
  }

  func resolve(completion: @escaping (BuzzPushResolution?) -> Void) {
    let communities = loadCommunities().filter {
      $0.pubkey?.isEmpty == false && loadPrivateKey(communityID: $0.id) != nil
    }
    guard !communities.isEmpty else { completion(nil); return }
    let group = DispatchGroup()
    let lock = NSLock()
    var candidates: [(BuzzPushResolution, VerifiedNostrEvent, BuzzPushCommunity)] = []
    for community in communities {
      group.enter()
      query(community) { candidate in
        if let candidate {
          lock.lock(); candidates.append((candidate.0, candidate.1, community)); lock.unlock()
        }
        group.leave()
      }
    }
    group.notify(queue: .global(qos: .userInitiated)) { [weak self] in
      guard let self else { return }
      let newest = candidates.max {
        $0.1.createdAt == $1.1.createdAt ? $0.1.id < $1.1.id : $0.1.createdAt < $1.1.createdAt
      }
      for candidate in candidates {
        self.defaults?.set(candidate.1.createdAt, forKey: self.watermarkKey(candidate.2.id))
      }
      completion(newest?.0)
    }
  }

  private func query(
    _ community: BuzzPushCommunity,
    completion: @escaping ((BuzzPushResolution, VerifiedNostrEvent)?) -> Void
  ) {
    guard let privateKey = loadPrivateKey(communityID: community.id), let pubkey = community.pubkey else {
      completion(nil); return
    }
    var filter: [String: Any] = ["kinds": [9, 40002, 45001, 45003], "#p": [pubkey], "limit": 10]
    let watermark = defaults?.integer(forKey: watermarkKey(community.id)) ?? 0
    if watermark > 0 { filter["since"] = watermark + 1 }
    guard let body = try? JSONSerialization.data(withJSONObject: [filter]) else { completion(nil); return }
    let url = URL(string: "/query", relativeTo: community.relayURL)!
    var request = URLRequest(url: url)
    request.httpMethod = "POST"; request.httpBody = body; request.timeoutInterval = 8
    request.setValue("application/json", forHTTPHeaderField: "Content-Type")
    guard let auth = try? NostrHTTPAuth.authorizationHeader(
      url: url, method: "POST", body: body, privateKeyHex: privateKey
    ) else { completion(nil); return }
    request.setValue(auth, forHTTPHeaderField: "Authorization")
    session.dataTask(with: request) { data, response, _ in
      guard let response = response as? HTTPURLResponse, (200..<300).contains(response.statusCode),
        let data, let events = try? JSONDecoder().decode([VerifiedNostrEvent].self, from: data)
      else { completion(nil); return }
      completion(Self.decodeResolution(events: events.filter { $0.hasValidIDAndSignature() }, community: community))
    }.resume()
  }

  private static func decodeResolution(
    events: [VerifiedNostrEvent], community: BuzzPushCommunity
  ) -> (BuzzPushResolution, VerifiedNostrEvent)? {
    guard let mine = community.pubkey?.lowercased() else { return nil }
    let event = events.filter {
      $0.pubkey.lowercased() != mine && [9, 40002, 45001, 45003].contains($0.kind)
    }.sorted {
      $0.createdAt == $1.createdAt ? $0.id < $1.id : $0.createdAt > $1.createdAt
    }.first
    guard let event else { return nil }
    let body = previewBody(event.content)
    guard !body.isEmpty else { return nil }
    let channel = event.tags.first { $0.count >= 2 && $0[0] == "h" }?[1]
    return (BuzzPushResolution(
      title: shortPubkey(event.pubkey), body: body, subtitle: community.name,
      threadIdentifier: channel ?? community.id
    ), event)
  }

  private static func previewBody(_ content: String) -> String {
    var result = content.replacingOccurrences(of: #"```[\s\S]*?```"#, with: "[code]", options: .regularExpression)
    result = result.replacingOccurrences(of: #"`([^`]*)`"#, with: "$1", options: .regularExpression)
    result = result.replacingOccurrences(of: #"!?\[([^\]]*)\]\([^)]*\)"#, with: "$1", options: .regularExpression)
    result = result.replacingOccurrences(of: #"https?://\S+"#, with: "[link]", options: .regularExpression)
    result = result.replacingOccurrences(of: #"\s+"#, with: " ", options: .regularExpression).trimmingCharacters(in: .whitespacesAndNewlines)
    return result.count > 180 ? String(result.prefix(177)).trimmingCharacters(in: .whitespacesAndNewlines) + "…" : result
  }

  private static func shortPubkey(_ pubkey: String) -> String {
    pubkey.count > 8 ? String(pubkey.prefix(8)) + "…" : pubkey
  }

  private func watermarkKey(_ id: String) -> String { "buzz.push.watermark.\(id)" }

  private func loadPrivateKey(communityID: String) -> String? {
    var query: [String: Any] = [
      kSecClass as String: kSecClassGenericPassword,
      kSecAttrService as String: "buzz.push.nse.signing",
      kSecAttrAccount as String: communityID,
      kSecReturnData as String: true,
      kSecMatchLimit as String: kSecMatchLimitOne,
    ]
    if let keychainAccessGroup, !keychainAccessGroup.isEmpty { query[kSecAttrAccessGroup as String] = keychainAccessGroup }
    var item: CFTypeRef?
    guard SecItemCopyMatching(query as CFDictionary, &item) == errSecSuccess,
      let data = item as? Data else { return nil }
    return String(data: data, encoding: .utf8)
  }

  private func loadCommunities() -> [BuzzPushCommunity] {
    guard let appGroupIdentifier,
      let container = FileManager.default.containerURL(forSecurityApplicationGroupIdentifier: appGroupIdentifier),
      let data = try? Data(contentsOf: container.appendingPathComponent("push-communities.json")),
      let decoded = try? JSONDecoder().decode(BuzzPushSnapshot.self, from: data)
    else { return [] }
    return decoded.communities
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
