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
    // The error wraps inside the field headings' margins at every width.
    let taken = "An open channel named \"release-notes-for-every-team\" already exists"
    for (width, error) in [(390.0, "Try again"), (320.0, taken), (820.0, taken)] {
      controller.tableView.frame.size.width = width
      controller.showError(error)
      controller.tableView.layoutIfNeeded()
      let heading = controller.tableView.headerView(forSection: 0)!.textLabel!
      let inset = heading.convert(heading.bounds, to: controller.tableView).minX
      let footer = controller.tableView.footerView(forSection: 2)!
      let label = labels(in: footer).first!
      let frame = label.convert(label.bounds, to: controller.tableView)
      precondition(label.text == error && !footer.accessibilityElementsHidden, "VoiceOver must reach the error")
      precondition(abs(frame.minX - inset) < 0.5, "Error must start where the field headings start")
      precondition(frame.maxX <= width - inset + 0.5, "Error must keep the same margin on its far side")
      precondition(label.textAlignment == .natural && label.numberOfLines == 0)
      if width == 320 { precondition(frame.height > label.font.lineHeight * 1.5, "A long error must wrap") }
    }
    precondition(controller.tableView.tableFooterView == nil, "UIKit does not lay out a table footer's x or width")
    print("Create-channel accessibility passed: fields own labels before and after retry; the error aligns with them")
  }

  private static func labels(in view: UIView) -> [UILabel] {
    (view as? UILabel).map { [$0] } ?? view.subviews.flatMap(labels)
  }
}
