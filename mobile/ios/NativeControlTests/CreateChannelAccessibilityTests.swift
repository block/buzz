import Flutter
import UIKit

@main
struct CreateChannelAccessibilityTests {
  @MainActor static func main() {
    let controller = NativeCreateChannelViewController(configuration: [:])
    controller.loadViewIfNeeded()
    controller.tableView.frame = CGRect(x: 0, y: 0, width: 390, height: 844)
    controller.tableView.reloadData()
    controller.tableView.layoutIfNeeded()
    for section in 0..<2 {
      let header = controller.tableView.headerView(forSection: section)!
      precondition(header.accessibilityElementsHidden, "Field heading must not add a VoiceOver stop")
      precondition(!header.isAccessibilityElement)
      precondition(header.textLabel?.isAccessibilityElement == false)
      let cell = controller.tableView.cellForRow(at: IndexPath(row: 0, section: section))!
      let field = cell.contentView.subviews.first { $0 is UITextField || $0 is UITextView }!
      precondition(field.accessibilityLabel == (section == 0 ? "Channel name" : "Description, optional"))
      precondition(!field.accessibilityElementsHidden)
    }
    controller.showError("Try again")
    controller.tableView.reloadData()
    controller.tableView.layoutIfNeeded()
    for section in 0..<2 {
      precondition(controller.tableView.headerView(forSection: section)!.accessibilityElementsHidden)
    }
    print("Create-channel accessibility passed: fields own labels before and after retry")
  }
}
