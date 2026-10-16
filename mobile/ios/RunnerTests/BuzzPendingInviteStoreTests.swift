import XCTest

@testable import Buzz

/// Exercises the store the App Clip writes and the full app drains. Items use
/// a test-only Keychain service and defaults suite so they never collide with
/// a real pending invite on the simulator.
final class BuzzPendingInviteStoreTests: XCTestCase {
  private var suiteName: String!
  private var defaults: UserDefaults!

  override func setUpWithError() throws {
    suiteName = "BuzzPendingInviteStoreTests.\(UUID().uuidString)"
    defaults = try XCTUnwrap(UserDefaults(suiteName: suiteName))
  }

  override func tearDown() {
    makeStore().clear()
    defaults.removePersistentDomain(forName: suiteName)
  }

  private func makeStore() -> BuzzPendingInviteStore {
    BuzzPendingInviteStore(
      appGroupIdentifier: nil,
      keychainService: "buzz.tests.pending-invite",
      userDefaults: defaults
    )
  }

  func testSaveWritesBothCopiesAndTakeClearsBoth() throws {
    let store = makeStore()
    let invite = BuzzPendingInvite(url: "https://relay.example.com/invite/abc", savedAt: Date(timeIntervalSince1970: 1_700_000_000))
    try store.save(invite)
    XCTAssertNotNil(defaults.data(forKey: BuzzPendingInviteStore.appGroupKey), "App Group fallback copy written")
    XCTAssertEqual(try store.peek(), invite)

    XCTAssertEqual(try store.take(), invite)
    XCTAssertNil(try store.peek(), "take() clears the Keychain copy")
    XCTAssertNil(defaults.data(forKey: BuzzPendingInviteStore.appGroupKey), "take() clears the App Group copy")
    XCTAssertNil(try store.take(), "a second take() finds nothing to replay")
  }

  func testSaveReplacesEarlierInvite() throws {
    let store = makeStore()
    try store.save(BuzzPendingInvite(url: "https://relay.example.com/invite/first", savedAt: Date()))
    let second = BuzzPendingInvite(url: "https://relay.example.com/invite/second", savedAt: Date())
    try store.save(second)
    XCTAssertEqual(try store.take(), second)
  }

  func testFallsBackToAppGroupCopyWhenKeychainIsEmpty() throws {
    let store = makeStore()
    let invite = BuzzPendingInvite(url: "https://relay.example.com/invite/abc", savedAt: Date(timeIntervalSince1970: 1_700_000_000))
    // Simulate an iOS < 15.4 install where only the group container migrated.
    defaults.set(try JSONEncoder().encode(invite), forKey: BuzzPendingInviteStore.appGroupKey)
    XCTAssertEqual(try store.take(), invite)
    XCTAssertNil(defaults.data(forKey: BuzzPendingInviteStore.appGroupKey))
  }

  func testCorruptAppGroupRecordThrowsAndClearDropsIt() {
    let store = makeStore()
    defaults.set(Data("not json".utf8), forKey: BuzzPendingInviteStore.appGroupKey)
    XCTAssertThrowsError(try store.take())
    store.clear()
    XCTAssertNil(defaults.data(forKey: BuzzPendingInviteStore.appGroupKey))
  }
}
