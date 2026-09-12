import BuzzPushKit
import Foundation
import Intents
import Security
import UserNotifications

final class NotificationService: UNNotificationServiceExtension {
  private var contentHandler: ((UNNotificationContent) -> Void)?
  private var bestAttemptContent: UNMutableNotificationContent?
  private var restrictedFallbackContent: UNMutableNotificationContent?
  private var restrictionFenceAtStart = BuzzAgeRestrictionFence.initial
  private let communicationPresenter = BuzzCommunicationNotificationPresenter()
  private let interactionDeletionDeadline = BuzzInteractionDeletionDeadline(
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
  private lazy var appGroupIdentifier =
    Bundle.main.object(
      forInfoDictionaryKey: "BuzzAppGroupIdentifier"
    ) as? String
  private lazy var resolver: BuzzPushNotificationResolving = {
    let keychainAccessGroup =
      Bundle.main.object(
        forInfoDictionaryKey: "BuzzKeychainAccessGroup"
      ) as? String
    return BuzzPushNotificationResolver(
      session: .shared,
      loadCommunitiesData: { [self] in
        Self.loadPushSnapshotData(appGroupIdentifier: self.appGroupIdentifier)
      },
      loadPrivateKey: { communityID in
        Self.loadPrivateKey(
          communityID: communityID,
          keychainAccessGroup: keychainAccessGroup
        )
      },
      loadPresentationCacheData: { [self] in
        Self.loadPushSnapshotData(appGroupIdentifier: self.appGroupIdentifier)
      }
    )
  }()

  override func didReceive(
    _ request: UNNotificationRequest,
    withContentHandler contentHandler: @escaping (UNNotificationContent) -> Void
  ) {
    self.contentHandler = contentHandler
    restrictionFenceAtStart = Self.loadRestrictionFence(
      appGroupIdentifier: appGroupIdentifier
    )
    restrictedFallbackContent = Self.restrictedFallback(from: request.content)
    guard let content = request.content.mutableCopy() as? UNMutableNotificationContent else {
      finish(request.content)
      return
    }
    bestAttemptContent = content
    var cleanUserInfo = content.userInfo
    cleanUserInfo.removeValue(forKey: BuzzPushNavigationTarget.userInfoKey)
    content.userInfo = cleanUserInfo

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
        if let navigationTarget = resolution.navigationTarget {
          var userInfo = content.userInfo
          userInfo[BuzzPushNavigationTarget.userInfoKey] = navigationTarget.userInfoValue
          content.userInfo = userInfo
        }
        self.bestAttemptContent = content
        self.communicationPresenter.present(
          ordinaryContent: content,
          resolution: resolution,
          isStillAllowed: { [weak self] in
            self?.restrictionFenceIsUnchanged() ?? false
          },
          onDeletionFailure: { [weak self] _ in
            self?.activateRestrictionFence()
          }
        ) { [weak self] specializedContent in
          self?.finish(specializedContent)
        }
        return
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
    let handedOff = Self.handoffIfRestrictionFenceUnchanged(
      appGroupIdentifier: appGroupIdentifier,
      since: restrictionFenceAtStart
    ) {
      contentHandler(content)
    }
    guard !handedOff else {
      return
    }

    interactionDeletionDeadline.deleteAll { [weak self, restrictedFallbackContent] error in
      if error != nil {
        self?.activateRestrictionFence()
      }
      let center = UNUserNotificationCenter.current()
      center.removeAllDeliveredNotifications()
      center.removeAllPendingNotificationRequests()
      contentHandler(restrictedFallbackContent ?? Self.restrictedFallback(from: content))
      // The handler queues delivery, so purge again after handing back only the
      // privacy-safe fallback. The persisted fence protects every later finish.
      center.removeAllDeliveredNotifications()
      center.removeAllPendingNotificationRequests()
    }
  }

  private func restrictionFenceIsUnchanged() -> Bool {
    !Self.loadRestrictionFence(
      appGroupIdentifier: appGroupIdentifier
    ).requiresDiscard(since: restrictionFenceAtStart)
  }

  private static func handoffIfRestrictionFenceUnchanged(
    appGroupIdentifier: String?,
    since earlier: BuzzAgeRestrictionFence,
    handoff: () -> Void
  ) -> Bool {
    guard let appGroupIdentifier,
      let container = FileManager.default.containerURL(
        forSecurityApplicationGroupIdentifier: appGroupIdentifier
      )
    else { return false }
    do {
      return try BuzzAgeRestrictionFenceStore(containerURL: container)
        .performIfUnchanged(since: earlier, handoff)
    } catch {
      return false
    }
  }

  private func activateRestrictionFence() {
    guard let appGroupIdentifier,
      let container = FileManager.default.containerURL(
        forSecurityApplicationGroupIdentifier: appGroupIdentifier
      )
    else { return }
    try? BuzzAgeRestrictionFenceStore(containerURL: container).begin()
  }

  private static func restrictedFallback(
    from content: UNNotificationContent
  ) -> UNMutableNotificationContent {
    let fallback =
      (content.mutableCopy() as? UNMutableNotificationContent)
      ?? UNMutableNotificationContent()
    fallback.title = "Buzz"
    fallback.subtitle = ""
    fallback.body = "Open Buzz to view this message."
    fallback.threadIdentifier = ""
    var userInfo = fallback.userInfo
    userInfo.removeValue(forKey: BuzzPushNavigationTarget.userInfoKey)
    fallback.userInfo = userInfo
    return fallback
  }

  private static func loadRestrictionFence(
    appGroupIdentifier: String?
  ) -> BuzzAgeRestrictionFence {
    guard let appGroupIdentifier,
      let container = FileManager.default.containerURL(
        forSecurityApplicationGroupIdentifier: appGroupIdentifier
      )
    else { return .unavailable }
    return BuzzAgeRestrictionFenceStore(containerURL: container).current()
  }

  private static func loadPrivateKey(
    communityID: String,
    keychainAccessGroup: String?
  ) -> String? {
    var query: [String: Any] = [
      kSecClass as String: kSecClassGenericPassword,
      kSecAttrService as String: "buzz.push.nse.signing",
      kSecAttrAccount as String: communityID,
      kSecReturnData as String: true,
      kSecMatchLimit as String: kSecMatchLimitOne,
    ]
    if let keychainAccessGroup, !keychainAccessGroup.isEmpty {
      query[kSecAttrAccessGroup as String] = keychainAccessGroup
    }
    var item: CFTypeRef?
    guard SecItemCopyMatching(query as CFDictionary, &item) == errSecSuccess,
      let data = item as? Data
    else { return nil }
    return String(data: data, encoding: .utf8)
  }

  private static func loadPushSnapshotData(appGroupIdentifier: String?) -> Data? {
    loadAppGroupData(
      fileName: BuzzPushPresentationCacheStore.fileName,
      appGroupIdentifier: appGroupIdentifier,
      maximumBytes: BuzzPushPresentationCacheStore.maximumSnapshotBytes
    )
  }

  private static func loadAppGroupData(
    fileName: String,
    appGroupIdentifier: String?,
    maximumBytes: Int? = nil
  ) -> Data? {
    guard let appGroupIdentifier,
      let container = FileManager.default.containerURL(
        forSecurityApplicationGroupIdentifier: appGroupIdentifier
      )
    else { return nil }
    let fileURL = container.appendingPathComponent(fileName)
    if let maximumBytes {
      guard let values = try? fileURL.resourceValues(forKeys: [.fileSizeKey]),
        let fileSize = values.fileSize,
        fileSize <= maximumBytes
      else { return nil }
    }
    return try? Data(contentsOf: fileURL)
  }
}
