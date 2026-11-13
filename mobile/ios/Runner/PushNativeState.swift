import BuzzPushKit
import Flutter
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

/// Carries replies typed into notifications to Flutter, which sends them, and
/// reports each outcome on the notification side. See
/// docs/adr/0004-notification-reply.md.
final class BuzzNotificationReplyBridge {
  /// Longest wait for Flutter to send before the reply counts as not sent.
  /// iOS gives a background notification action about 30 seconds.
  static let sendTimeout: TimeInterval = 25
  static let replyNotSentThread = "buzz.reply-not-sent"

  private let buffer = BuzzNotificationReplyBuffer()
  private let selectedCommunityDefaults: UserDefaults?
  /// Tells Flutter a reply is waiting; Flutter also takes waiting replies
  /// when its handler starts, so a call made before then is harmless.
  var notifyFlutter: (() -> Void)?

  init(appGroupIdentifier: String?) {
    selectedCommunityDefaults = appGroupIdentifier.flatMap(UserDefaults.init(suiteName:))
  }

  static func registerCategory(with center: UNUserNotificationCenter = .current()) {
    let reply = UNTextInputNotificationAction(
      identifier: BuzzNotificationReplyContext.actionIdentifier,
      title: "Reply",
      // The app reads its keychain-held keys to send.
      options: [.authenticationRequired],
      textInputButtonTitle: "Send",
      textInputPlaceholder: ""
    )
    center.setNotificationCategories([
      UNNotificationCategory(
        identifier: BuzzNotificationReplyContext.categoryIdentifier,
        actions: [reply],
        intentIdentifiers: [],
        options: []
      )
    ])
  }

  /// Handles a Reply action. Returns false for any other response.
  func handle(
    _ response: UNNotificationResponse,
    center: UNUserNotificationCenter = .current(),
    completion: @escaping () -> Void
  ) -> Bool {
    guard response.actionIdentifier == BuzzNotificationReplyContext.actionIdentifier else {
      return false
    }
    let content = response.notification.request.content
    guard let text = (response as? UNTextInputNotificationResponse)?.userText,
      let request = BuzzNotificationReplyRequest.make(
        id: UUID().uuidString,
        text: text,
        userInfo: content.userInfo,
        threadIdentifier: content.threadIdentifier
      )
    else {
      completion()
      return true
    }
    let notificationID = response.notification.request.identifier
    // Runs on the main queue, like every caller below; ends the task once.
    var backgroundTask = UIBackgroundTaskIdentifier.invalid
    let endBackgroundTask = {
      guard backgroundTask != .invalid else { return }
      UIApplication.shared.endBackgroundTask(backgroundTask)
      backgroundTask = .invalid
    }
    backgroundTask = UIApplication.shared.beginBackgroundTask { [buffer] in
      // iOS requires the task to end before this handler returns; the
      // unsent reply is still reported if the app gets the time.
      buffer.complete(id: request.id, sent: false)
      endBackgroundTask()
    }
    buffer.add(request) { sent in
      Self.finish(request, sent: sent, notificationID: notificationID, center: center) {
        DispatchQueue.main.async {
          completion()
          endBackgroundTask()
        }
      }
    }
    DispatchQueue.main.asyncAfter(deadline: .now() + Self.sendTimeout) { [buffer] in
      buffer.complete(id: request.id, sent: false)
    }
    notifyFlutter?()
    return true
  }

  /// Handles the reply methods on the push channel. Returns false for others.
  func handle(_ call: FlutterMethodCall, result: @escaping FlutterResult) -> Bool {
    switch call.method {
    case "takePendingNotificationReplies":
      result(buffer.take().map(Self.flutterArguments))
    case "completeNotificationReply":
      guard let arguments = call.arguments as? [String: Any],
        let id = arguments["id"] as? String,
        let sent = arguments["sent"] as? Bool
      else {
        result(
          FlutterError(code: "invalid_arguments", message: "Expected id and sent.", details: nil))
        return true
      }
      buffer.complete(id: id, sent: sent)
      result(nil)
    case "setSelectedCommunity":
      BuzzSelectedCommunity.set(call.arguments as? String, in: selectedCommunityDefaults)
      result(nil)
    default:
      return false
    }
    return true
  }

  private static func flutterArguments(_ request: BuzzNotificationReplyRequest) -> [String: String] {
    var arguments = request.target.flutterArguments
    arguments["id"] = request.id
    arguments["text"] = request.text
    arguments["senderPubkey"] = request.context.senderPubkey
    arguments["replyRootId"] = request.context.threadRootID
    arguments["messageCreatedAt"] = String(request.context.messageCreatedAt)
    return arguments
  }

  /// A sent reply clears the notifications it marked read: the notified one
  /// and earlier ones in the same conversation. An unsent one is replaced by a
  /// "Reply not sent" notification that keeps the typed text and opens the
  /// conversation.
  private static func finish(
    _ request: BuzzNotificationReplyRequest,
    sent: Bool,
    notificationID: String,
    center: UNUserNotificationCenter,
    done: @escaping () -> Void
  ) {
    guard sent else {
      let content = UNMutableNotificationContent()
      content.title = "Reply not sent"
      content.body = request.text
      // Grouped apart, so clearing a conversation never removes it.
      content.threadIdentifier = Self.replyNotSentThread
      content.userInfo = [BuzzPushNavigationTarget.userInfoKey: request.target.userInfoValue]
      center.add(
        UNNotificationRequest(
          identifier: "buzz.reply-not-sent.\(request.id)", content: content, trigger: nil)
      ) { _ in done() }
      return
    }
    center.getDeliveredNotifications { delivered in
      let ids = delivered.compactMap { notification -> String? in
        let identifier = notification.request.identifier
        let content = notification.request.content
        guard identifier != notificationID else { return identifier }
        // Only notifications that offer Reply know where and when their
        // message was; the replied conversation's earlier ones were read.
        guard !request.threadIdentifier.isEmpty,
          content.threadIdentifier == request.threadIdentifier,
          let context = BuzzNotificationReplyContext.decodeIfPresent(from: content.userInfo),
          context.threadRootID == request.context.threadRootID,
          context.messageCreatedAt <= request.context.messageCreatedAt
        else { return nil }
        return identifier
      }
      center.removeDeliveredNotifications(withIdentifiers: ids)
      done()
    }
  }
}
