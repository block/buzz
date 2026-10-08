import Flutter
import UIKit

@main
struct AddMembersIdentityTests {
  @MainActor static func main() {
    let keys = [String(repeating: "a", count: 64), String(repeating: "b", count: 64)]
    let labels = ["Scout · aaaaaaaa", "Scout · bbbbbbbb"]
    let users: [[String: Any]] = zip(keys, labels).map { key, label in
      ["pubkey": key, "name": label, "detail": "Same owner", "agent": true, "initial": "S", "avatarKey": key]
    }
    var selected: [String] = []
    var avatarReplies: [String: (Data?) -> Void] = [:]
    let avatar = UIGraphicsImageRenderer(size: CGSize(width: 40, height: 40)).image { context in
      UIColor.blue.setFill()
      context.fill(CGRect(x: 0, y: 0, width: 40, height: 40))
    }.pngData()
    let picker = NativeAddMembersViewController(
      state: ["users": users, "selected": [], "query": "", "loading": false],
      loadAvatar: { key, _, reply in avatarReplies[key] = reply },
      send: { event, values in
        if event == "toggle", let key = values["pubkey"] as? String { selected.append(key) }
      }
    )
    picker.loadViewIfNeeded()
    for index in keys.indices {
      let path = IndexPath(row: index, section: 1)
      let cell = picker.tableView(picker.tableView, cellForRowAt: path)
      let content = cell.contentConfiguration as? UIListContentConfiguration
      precondition(content?.text == labels[index], "Visible identity must retain its qualifier")
      precondition(cell.isAccessibilityElement, "The row owns accessibility")
      precondition(cell.contentView.accessibilityElementsHidden, "No duplicate content stop")
      precondition(cell.accessibilityLabel == "\(labels[index]), Same owner", "VoiceOver must retain the qualifier")
      avatarReplies[keys[index]]?(avatar)
      precondition(cell.accessibilityLabel == "\(labels[index]), Same owner", "Avatar loading retains identity")
      precondition(cell.contentView.accessibilityElementsHidden, "Avatar loading keeps one accessibility owner")
      picker.tableView(picker.tableView, didSelectRowAt: path)
    }
    precondition(selected == keys, "Each distinct label must select its own pubkey")
    picker.update(["users": users, "selected": [users[1]], "query": "", "loading": false])
    let chosen = picker.tableView(picker.tableView, cellForRowAt: IndexPath(row: 0, section: 0))
    precondition(chosen.accessibilityLabel == "\(labels[1]), Same owner", "Selected identity retains its qualifier")
    precondition(chosen.accessibilityTraits.contains(.selected), "Selection is exposed to VoiceOver")
    print("Native member picker identity and accessibility passed")
  }
}
