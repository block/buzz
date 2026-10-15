import Foundation

/// One notification's presentation deadline, independent of the lifetime of its fetches.
final class BuzzNotificationEnrichmentOperation {
  typealias Refresh = (
    Bool, Bool, @escaping (VerifiedNostrEvent?, VerifiedNostrEvent?, VerifiedNostrEvent?) -> Void
  ) -> Void
  private let event: VerifiedNostrEvent
  private let community: PushLeaseCommunity
  private let scope: BuzzNotificationCacheScope
  private let cache: BuzzNotificationCache
  typealias LoadAvatar = (
    String, BuzzNotificationCacheScope, @escaping (Result<Data, Error>) -> Void
  ) -> Void
  typealias ScheduleDeadline = (DispatchQueue, @escaping () -> Void) -> Void
  private let loadAvatar: LoadAvatar?
  private let scheduleDeadline: ScheduleDeadline
  private let now: () -> Date
  private let isCurrent: () -> Bool
  private let refresh: Refresh
  private var completion: ((BuzzPushResolution?) -> Void)?
  private let queue = DispatchQueue(label: "buzz.notification.enrichment")
  private var profile: BuzzNotificationMetadata?
  private var channel: BuzzNotificationMetadata?
  private var membership: BuzzNotificationMetadata?
  private var avatar: BuzzNotificationAvatar?

  init(
    event: VerifiedNostrEvent, community: PushLeaseCommunity, scope: BuzzNotificationCacheScope,
    cache: BuzzNotificationCache, avatarLoader: BuzzNotificationAvatarLoader?,
    now: @escaping () -> Date, isCurrent: @escaping () -> Bool,
    loadAvatar: LoadAvatar? = nil,
    scheduleDeadline: @escaping ScheduleDeadline = { queue, action in
      queue.asyncAfter(deadline: .now() + 1.5, execute: DispatchWorkItem(block: action))
    },
    refresh: @escaping Refresh, completion: @escaping (BuzzPushResolution?) -> Void
  ) {
    self.event = event
    self.community = community
    self.scope = scope
    self.cache = cache
    self.loadAvatar =
      loadAvatar
      ?? avatarLoader.map { loader in
        { source, scope, completion in
          loader.load(source: source, scope: scope, completion: completion)
        }
      }
    self.scheduleDeadline = scheduleDeadline
    self.now = now
    self.isCurrent = isCurrent
    self.refresh = refresh
    self.completion = completion
  }

  func start() {
    queue.async { [self] in
      scheduleDeadline(queue) { [self] in deliver() }
      readCache { [self] in
        let refreshProfile = profile?.isFresh(at: now()) != true
        let refreshChannel =
          channelID != nil
          && (channel?.isFresh(at: now()) != true || membership?.isFresh(at: now()) != true)
        if refreshProfile || refreshChannel {
          refresh(refreshProfile, refreshChannel) { [self] profile, channel, membership in
            DispatchQueue.global(qos: .utility).async { [self] in
              if isCurrent() {
                if let profile {
                  do { try cache.save(event: profile, scope: scope) } catch { report(error) }
                }
                do {
                  try cache.save(events: [channel, membership].compactMap { $0 }, scope: scope)
                } catch { report(error) }
              }
              readCache { [self] in fetchAvatar() }
            }
          }
        } else {
          fetchAvatar()
        }
      }
    }
  }

  private var channelID: String? {
    BuzzPushNotificationResolver.tagValue("h", in: event)
  }

  private func readCache(completion: @escaping () -> Void) {
    DispatchQueue.global(qos: .utility).async { [self] in
      var loaded = BuzzNotificationCache.Presentation()
      do {
        loaded = try cache.presentation(
          scope: scope, sender: event.pubkey,
          channelID: channelID, authority: community.relayMetadataPubkey?.lowercased())
      } catch { report(error) }
      let snapshot = loaded
      queue.async { [self] in
        profile = snapshot.profile
        avatar = snapshot.avatar
        channel = snapshot.channel
        membership = snapshot.membership
        completion()
      }
    }
  }

  private var avatarIsFresh: Bool {
    guard let avatar else { return false }
    let age = now().timeIntervalSince(avatar.savedAt)
    return age >= 0 && age < BuzzNotificationMetadata.freshness
  }

  private func fetchAvatar() {
    guard isCurrent(), let profile, let source = BuzzNotificationCache.picture(profile.event),
      avatar?.source != source || !avatarIsFresh, let loadAvatar
    else {
      deliver()
      return
    }
    // Decoding and persistence cannot occupy the delivery deadline's queue.
    DispatchQueue.global(qos: .utility).async { [self] in
      loadAvatar(source, scope) { [self] result in
        DispatchQueue.global(qos: .utility).async { [self] in
          switch result {
          case .success(let data):
            do {
              try cache.saveAvatar(
                data, source: source, profileID: profile.event.id,
                pubkey: event.pubkey, scope: scope)
            } catch { report(error) }
          case .failure(let error): report(error)
          }
          readCache { [self] in deliver() }
        }
      }
    }
  }

  private func deliver() {
    guard let completion else { return }
    self.completion = nil
    guard isCurrent() else {
      completion(nil)
      return
    }
    let cachedProfile = profile.map { record in
      let metadata = BuzzPushPresentationCacheStore.profileMetadata(record.event)
      return BuzzPushCachedProfile(
        communityID: community.id, relayOrigin: scope.relayOrigin,
        pubkey: event.pubkey, displayName: metadata.displayName,
        pictureHash: metadata.pictureHash, avatarPNG: avatar?.data,
        eventID: record.event.id, eventCreatedAt: record.event.createdAt,
        cachedAt: Int(record.checkedAt.timeIntervalSince1970))
    }
    let cachedChannel = community.relayMetadataPubkey.flatMap {
      BuzzPushNotificationResolver.ephemeralChannel(
        metadataEvent: channel?.event, membershipEvent: membership?.event, cached: nil,
        communityID: community.id, relayOrigin: scope.relayOrigin, relayMetadataPubkey: $0,
        cachedAt: Int(now().timeIntervalSince1970))
    }
    completion(
      BuzzPushNotificationResolver.makeResolution(
        event: event, community: community, profile: cachedProfile, channel: cachedChannel))
  }

  private func report(_ error: Error) {
    // These records are a rebuildable cache. Failure is observable, and the
    // next notification retries rather than treating it as an authoritative empty profile.
    NSLog("Notification enrichment failed: %@", String(describing: error))
  }
}
