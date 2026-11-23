import UIKit
import XCTest
@testable import Buzz

final class HomeScreenShortcutsTests: XCTestCase {
  private var savedItems: [UIApplicationShortcutItem]?

  @MainActor
  override func setUp() {
    super.setUp()
    savedItems = UIApplication.shared.shortcutItems
  }

  @MainActor
  override func tearDown() {
    UIApplication.shared.shortcutItems = savedItems
    super.tearDown()
  }

  @MainActor
  private func publish(_ types: [String]) {
    UIApplication.shared.shortcutItems = types.map {
      UIApplicationShortcutItem(type: $0, localizedTitle: $0)
    }
  }

  @MainActor
  private func open(_ link: String) -> Bool {
    HomeScreenShortcuts().open(URL(string: link)!)
  }

  @MainActor
  func testShortcutLinksAreIgnoredWhileNoQuickActionsArePublished() {
    // Signed out (or before Home first publishes), the app icon has no quick
    // actions. A link then must not queue an action for the next account.
    publish([])
    XCTAssertFalse(open("buzz://shortcut/new-chat"))
    XCTAssertFalse(open("buzz://shortcut/message-bestie"))
  }

  @MainActor
  func testShortcutLinksRunOnlyPublishedQuickActions() {
    publish(["new-chat"])
    XCTAssertTrue(open("buzz://shortcut/new-chat"))
    XCTAssertFalse(open("buzz://shortcut/message-bestie"))
  }

  @MainActor
  func testOtherLinksAreNotShortcuts() {
    publish(["new-chat", "new-dm", "message-bestie"])
    XCTAssertFalse(open("buzz://message?channel=c&id=e"))
  }
}
