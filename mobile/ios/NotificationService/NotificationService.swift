import BuzzPushKit
import Foundation
import Intents
import Security
import UserNotifications

final class NotificationService: UNNotificationServiceExtension {
  private var handoff: BuzzNotificationHandoff<UNNotificationContent>?
  private var resolvedScope: BuzzNotificationCacheScope?
  private var bestAttemptContent: UNMutableNotificationContent?
  private var restrictedFallbackContent: UNMutableNotificationContent?
  private lazy var interactionCleanup = BuzzInteractionCleanupRetry(
    containerURL: ageRestrictionContainerURL, deletion: interactionDeletionDeadline)
  private lazy var communicationPresenter = BuzzCommunicationNotificationPresenter(
    donate: { interaction, completion in interaction.donate(completion: completion) },
    deleteAllInteractions: { [cleanup = interactionCleanup] completion in
      cleanup.request(completion: completion)
    },
    updateContent: { content, intent in try content.updating(from: intent) }
  )
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
      notificationCache: notificationCache,
      avatarLoader: notificationAvatarLoader
    )
  }()

  private var notificationCacheRoot: URL? {
    FileManager.default.urls(for: .cachesDirectory, in: .userDomainMask).first?
      .appendingPathComponent("notification-enrichment-v1", isDirectory: true)
  }

  private lazy var notificationCache: BuzzNotificationCache? = {
    guard let directory = notificationCacheRoot else { return nil }
    let cache = BuzzNotificationCache(directory: directory, isCurrent: { [weak self] scope in
      self?.isCurrent(scope) == true
    })
    let scopes = Self.communities(appGroupIdentifier: appGroupIdentifier)
      .compactMap(BuzzNotificationCacheScope.init(community:))
    do { try cache.removeRevokedScopes(keeping: scopes) }
    catch { NSLog("Notification cache cleanup failed: %@", String(describing: error)) }
    return cache
  }()

  private lazy var notificationAvatarLoader: BuzzNotificationAvatarLoader? = {
    guard let directory = notificationCacheRoot else { return nil }
    let accessGroup = Bundle.main.object(forInfoDictionaryKey: "BuzzKeychainAccessGroup") as? String
    return BuzzNotificationAvatarLoader(directory: directory,
      privateKey: { Self.loadPrivateKey(communityID: $0, keychainAccessGroup: accessGroup) },
      isCurrent: { [weak self] in self?.isCurrent($0) == true })
  }()

  private func isCurrent(_ scope: BuzzNotificationCacheScope) -> Bool {
    let accessGroup = Bundle.main.object(forInfoDictionaryKey: "BuzzKeychainAccessGroup") as? String
    return Self.communities(appGroupIdentifier: appGroupIdentifier).contains {
      BuzzNotificationCacheScope(community: $0) == scope
    } && scope.matchesAccountKey(
      Self.loadPrivateKey(communityID: scope.communityID, keychainAccessGroup: accessGroup))
  }

  private static func communities(appGroupIdentifier: String?) -> [PushLeaseCommunity] {
    guard let data = loadPushSnapshotData(appGroupIdentifier: appGroupIdentifier),
      let snapshot = try? JSONDecoder().decode(PushLeaseSnapshot.self, from: data)
    else { return [] }
    return snapshot.communities
  }

  override func didReceive(
    _ request: UNNotificationRequest,
    withContentHandler contentHandler: @escaping (UNNotificationContent) -> Void
  ) {
    resolvedScope = nil
    handoff = BuzzNotificationHandoff(handler: contentHandler)
    guard let content = request.content.mutableCopy() as? UNMutableNotificationContent else {
      finish(request.content)
      return
    }
    // Workaround until https://github.com/block/buzz/pull/8431 lands.
    if content.sound == nil { content.sound = .default }
    restrictedFallbackContent = Self.restrictedFallback(from: content)
    bestAttemptContent = content
    var cleanUserInfo = content.userInfo
    cleanUserInfo.removeValue(forKey: BuzzPushNavigationTarget.userInfoKey)
    content.userInfo = cleanUserInfo

    resolver.resolve { [weak self] resolution in
      guard let self else { return }
      if let resolution {
        guard let scope = resolution.notificationScope, self.isCurrent(scope) else {
          self.finish(Self.restrictedFallback(from: content))
          return
        }
        self.resolvedScope = scope
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
            guard let self else { return false }
            return self.isCurrent(scope) && !self.isAgeRestricted()
          },
          onDeletionFailure: { error in
            NSLog("Notification cleanup retry failed: %@", String(describing: error))
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
    handoff?.finish(
      resolvedScope.map { isCurrent($0) } == false
        ? Self.restrictedFallback(from: content) : content,
      restrictedFallback: restrictedFallbackContent ?? Self.restrictedFallback(from: content),
      handoffIfAllowed: { deliver in
        BuzzAgeRestrictionSession.handoffIfAllowed(
          containerURL: ageRestrictionContainerURL, deliver: deliver)
      }
    ) { [self] in
      // The service deadline cannot wait for Intents cleanup. The safe content
      // has already been handed back synchronously, including on expiration.
      let center = UNUserNotificationCenter.current()
      center.removeAllDeliveredNotifications()
      center.removeAllPendingNotificationRequests()
      interactionCleanup.request { error in
        if error != nil { NSLog("Notification cleanup failed; age access is unchanged.") }
        center.removeAllDeliveredNotifications()
        center.removeAllPendingNotificationRequests()
      }
    }
  }

  private var ageRestrictionContainerURL: URL? {
    appGroupIdentifier.flatMap {
      FileManager.default.containerURL(forSecurityApplicationGroupIdentifier: $0)
    }
  }

  private func isAgeRestricted() -> Bool {
    BuzzAgeRestrictionSession.isRestricted(containerURL: ageRestrictionContainerURL)
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
