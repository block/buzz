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
  /// Keeps each reply until the app reports on it, so a reply the app never
  /// finished, even across a relaunch, still becomes a draft.
  private let journal = BuzzNotificationReplyJournal(defaults: .standard)
  private let selectedCommunityDefaults: UserDefaults?
  /// Buffered replies whose community is gone: their notice is the only copy
  /// of their text, so they're settled only once it's posted. Main queue only.
  private var settleOnNotice: Set<String> = []
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
        threadIdentifier: content.threadIdentifier,
        deadline: Date().addingTimeInterval(Self.sendTimeout)
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
    backgroundTask = UIApplication.shared.beginBackgroundTask { [weak self] in
      // iOS requires the task to end before this handler returns; the
      // unsent reply is still reported if the app gets the time.
      self?.expire(request.id)
      endBackgroundTask()
    }
    journal.record(request.journalArguments)
    buffer.add(request) { [weak self] outcome in
      Self.finish(request, outcome: outcome, notificationID: notificationID, center: center) {
        posted in
        DispatchQueue.main.async {
          if self?.settleOnNotice.remove(request.id) != nil, posted {
            self?.journal.settle(id: request.id)
          }
          completion()
          endBackgroundTask()
        }
      }
    }
    DispatchQueue.main.asyncAfter(deadline: .now() + Self.sendTimeout) { [weak self] in
      self?.expire(request.id)
    }
    notifyFlutter?()
    return true
  }

  /// Handles the reply methods on the push channel. Returns false for others.
  func handle(_ call: FlutterMethodCall, result: @escaping FlutterResult) -> Bool {
    switch call.method {
    case "takePendingNotificationReplies":
      // Every buffered reply is in the journal, which also holds any a
      // previous launch never finished.
      _ = buffer.take()
      result(journal.take())
    case "markNotificationReplyPublishing":
      guard let id = call.arguments as? String else {
        result(FlutterError(code: "invalid_arguments", message: "Expected id.", details: nil))
        return true
      }
      // Once expired, the reply was reported as not sent with its text, so
      // it must not go out. Runs on the main queue, like expire.
      guard buffer.holds(id: id) else {
        result(false)
        return true
      }
      journal.markPublishing(id: id)
      result(true)
    case "completeNotificationReply":
      guard let arguments = call.arguments as? [String: Any],
        let id = arguments["id"] as? String,
        let sent = arguments["sent"] as? Bool
      else {
        result(
          FlutterError(code: "invalid_arguments", message: "Expected id and sent.", details: nil))
        return true
      }
      let outcome: BuzzNotificationReplyOutcome =
        sent
        ? .sent
        : arguments["unknown"] as? Bool == true
          ? .unknown
          : .notSent(draftThreadRootID: arguments["draftThreadRootId"] as? String)
      // The app reports only after the reply is sent, may have been sent, has
      // its draft saved, or has no community left to keep one in.
      let draftKept = arguments["noDraft"] as? Bool != true
      let recorded = journal.recorded(id: id)
      // One that may have gone out keeps no draft either, so its notice is
      // the only copy of its text until iOS accepts it.
      let settlesNow = draftKept && outcome != .unknown
      if !settlesNow { settleOnNotice.insert(id) }
      if buffer.complete(id: id, outcome: outcome) {
        if settlesNow { journal.settle(id: id) }
      } else {
        settleOnNotice.remove(id)
        // A notice that iOS refused leaves the reply for the next launch.
        finishUnbuffered(
          id: id, outcome: outcome, draftKept: draftKept, recorded: recorded
        ) { [journal] posted in
          if posted { journal.settle(id: id) }
        }
      }
      result(nil)
    case "setSelectedCommunity":
      BuzzSelectedCommunity.set(call.arguments as? String, in: selectedCommunityDefaults)
      result(nil)
    default:
      return false
    }
    return true
  }

  /// Ends a reply at its deadline. It stays in the journal until the app
  /// reports on it, so its text still becomes a draft, unless the app may
  /// already have published it.
  private func expire(_ id: String) {
    buffer.expire(id: id, mayHavePublished: journal.isPublishing(id: id))
  }

  /// Reports a reply this process no longer holds: one an earlier launch was
  /// given, or one that expired first. One that may have gone out gets its
  /// "Reply may not have sent" notification, which an earlier launch never
  /// posted; one that went out after all clears the notice its expiry posted.
  /// One whose community is gone keeps no draft, so it gets a "Reply not
  /// sent" notification with its text; its expiry's notice has the same
  /// identifier, so the text shows once. Calls [settle] once its notice is
  /// posted, saying whether iOS accepted it.
  private func finishUnbuffered(
    id: String, outcome: BuzzNotificationReplyOutcome, draftKept: Bool,
    recorded: [String: String]?, settle: @escaping (Bool) -> Void
  ) {
    switch outcome {
    case .sent:
      UNUserNotificationCenter.current().removeDeliveredNotifications(
        withIdentifiers: [Self.replyNotSentIdentifier(id)])
    case .unknown,
      .notSent where !draftKept:
      if let request = recorded.flatMap(BuzzNotificationReplyRequest.init(journalArguments:)) {
        Self.finish(
          request, outcome: outcome, notificationID: "", center: .current(), done: settle)
        return
      }
    case .notSent:
      // Its draft is saved, where its notice (if any) already points.
      break
    }
    settle(true)
  }

  private static func replyNotSentIdentifier(_ id: String) -> String {
    "buzz.reply-not-sent.\(id)"
  }

  /// A sent reply clears the notifications it marked read: the notified one
  /// and earlier ones in the same conversation. An unsent one is replaced by a
  /// "Reply not sent" notification that keeps the typed text and opens its
  /// draft; one that may have gone out says so instead, without a draft.
  /// [done] says whether iOS accepted the notice.
  private static func finish(
    _ request: BuzzNotificationReplyRequest,
    outcome: BuzzNotificationReplyOutcome,
    notificationID: String,
    center: UNUserNotificationCenter,
    done: @escaping (Bool) -> Void
  ) {
    guard outcome == .sent else {
      let content = UNMutableNotificationContent()
      var target = request.target
      switch outcome {
      case .notSent(let draftThreadRootID):
        content.title = "Reply not sent"
        if let draftThreadRootID {
          target = BuzzPushNavigationTarget(
            eventID: target.eventID, communityID: target.communityID,
            channelID: target.channelID, threadRootID: draftThreadRootID)
        }
      default:
        content.title = "Reply may not have sent"
      }
      content.body = request.text
      // Grouped apart, so clearing a conversation never removes it.
      content.threadIdentifier = Self.replyNotSentThread
      content.userInfo = [BuzzPushNavigationTarget.userInfoKey: target.userInfoValue]
      center.add(
        UNNotificationRequest(
          identifier: Self.replyNotSentIdentifier(request.id), content: content, trigger: nil)
      ) { error in done(error == nil) }
      return
    }
    center.getDeliveredNotifications { delivered in
      let ids = delivered.compactMap { notification -> String? in
        let identifier = notification.request.identifier
        let content = notification.request.content
        guard
          identifier == notificationID
            || request.clears(
              threadIdentifier: content.threadIdentifier, userInfo: content.userInfo)
        else { return nil }
        return identifier
      }
      center.removeDeliveredNotifications(withIdentifiers: ids)
      done(true)
    }
  }
}
