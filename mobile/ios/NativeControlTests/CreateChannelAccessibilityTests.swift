import Flutter
import UIKit

@main
struct CreateChannelAccessibilityTests {
  // The insets stand in for the navigation bar and the home indicator.
  private static let landscape = (CGSize(width: 852, height: 393), UIEdgeInsets(top: 56, left: 0, bottom: 21, right: 0))
  private static let shortPortrait = (CGSize(width: 375, height: 667), UIEdgeInsets(top: 64, left: 0, bottom: 34, right: 0))
  private static let smallPortrait = (CGSize(width: 320, height: 568), UIEdgeInsets(top: 56, left: 0, bottom: 0, right: 0))

  @MainActor static func main() {
    // The script pins the text size of both runs, so neither takes it from
    // the simulator's setting.
    if CommandLine.arguments.contains("UICTContentSizeCategoryAccessibilityXXXL") { return largeTextMain() }
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
    // The error wraps inside the field headings' margins at every width,
    // centred in them, a group's spacing below the rows. The last step narrows
    // the table under an unchanged error, as rotation does: nothing reloads
    // the section, so UIKit alone must rewrap the footer.
    let taken = "An open channel named \"release-notes-for-every-team\" already exists"
    let table = controller.tableView!
    for (width, error) in [(390.0, "Try again"), (820.0, taken), (320.0, taken)] {
      table.frame.size.width = width
      controller.showError(error)
      table.layoutIfNeeded()
      let heading = table.headerView(forSection: 0)!.textLabel!
      let inset = heading.convert(heading.bounds, to: table).minX
      let footer = table.footerView(forSection: 2)!
      let label = labels(in: footer).first!
      let frame = label.convert(label.bounds, to: table)
      precondition(label.text == error)
      // Hiding any view from the label up to the table would remove the
      // VoiceOver stop, as willDisplayHeaderView does for the headings.
      var ancestor: UIView? = label
      while let view = ancestor, view !== table {
        precondition(!view.accessibilityElementsHidden && !view.isHidden, "VoiceOver must reach the error")
        ancestor = view.superview
      }
      precondition(ancestor === table)
      // The label fills the headings' margins and centres each line in it,
      // so the text's centre is the table's.
      precondition(abs(frame.minX - inset) < 0.5 && abs(width - frame.maxX - inset) < 0.5, "Error must keep the field headings' margin on both sides")
      precondition(abs(frame.midX - width / 2) < 0.5, "Error must be centred in the table")
      precondition(label.textAlignment == .center && label.numberOfLines == 0, "Each line of the error must be centred")
      // The first group has no footer, so its footer's height is the space
      // UIKit leaves under a group.
      let rows = table.rectForRow(at: IndexPath(row: 1, section: 2))
      let spacing = table.rectForFooter(inSection: 0).height
      precondition(abs(frame.minY - rows.maxY - spacing) < 0.5, "Error must start a group's spacing below the rows")
      let needed = label.sizeThatFits(CGSize(width: frame.width, height: .greatestFiniteMagnitude)).height
      precondition(frame.height >= needed - 0.5, "The error must not be clipped")
      let footerRect = table.rectForFooter(inSection: 2)
      precondition(footerRect.insetBy(dx: 0, dy: -0.5).contains(frame), "The footer must hold the whole error")
      precondition(footerRect.maxY - frame.maxY > 4, "A scrolled-into-view error must not touch the bottom edge")
      if width == 320 { precondition(frame.height > label.font.lineHeight * 1.5, "A long error must wrap") }
    }
    precondition(table.tableFooterView == nil, "UIKit does not lay out a table footer's x or width")
    // The next Create tap clears the error while the request is out.
    let nameCell = table.cellForRow(at: IndexPath(row: 0, section: 0))!
    let name = nameCell.contentView.subviews.compactMap { $0 as? UITextField }.first!
    name.text = "release-notes"
    // sendActions(for:) needs a UIApplication, which this process does not have.
    for target in name.allTargets {
      for action in name.actions(forTarget: target, forControlEvent: .editingChanged) ?? [] {
        _ = (target as NSObject).perform(Selector(action))
      }
    }
    var submitted: [[String: Any]] = []
    controller.onCreate = { submitted.append($0) }
    let button = controller.navigationItem.rightBarButtonItem!
    precondition(button.isEnabled && button.title == "Create")
    _ = (button.target as! NSObject).perform(button.action!)
    table.layoutIfNeeded()
    precondition(submitted.count == 1 && submitted[0]["name"] as? String == "release-notes")
    precondition(table.footerView(forSection: 2) == nil, "A new attempt must clear the previous error")
    precondition(button.title == "Creating…" && !button.isEnabled)
    // A failure in landscape, where the footer starts below the fold, scrolls
    // the error into view.
    expectFailureScrolledIntoView(controller, screen: landscape, error: taken)
    precondition(button.title == "Create" && button.isEnabled)
    print("Create-channel accessibility passed: fields own labels before and after retry; the error is centred in their margins, clears on retry and scrolls into view")
  }

  // The script's second run, at the largest text size. Rows are then taller
  // than their estimated height, and a first failed create leaves the ones
  // below the fold on the estimate: every row in landscape, only the last on
  // a short portrait screen. A layout and scroll pass stops short by the
  // rows it has not yet brought on screen, so the Temporary type's third row
  // needs a third pass. That case uses the smallest screen: on the 375-point
  // one the type row measures at one of two heights from run to run, and
  // two passes are enough at one of them.
  @MainActor private static func largeTextMain() {
    let taken = "An open channel named \"general\" already exists"
    for (screen, temporary, error) in [(landscape, false, taken), (shortPortrait, false, taken), (smallPortrait, true, "Try again")] {
      let controller = NativeCreateChannelViewController(configuration: [:])
      controller.loadViewIfNeeded()
      let table = controller.tableView!
      table.reloadData()
      if temporary {
        // Pick Temporary through the type row's own menu.
        table.frame.size = screen.0
        table.contentInset = screen.1
        table.scrollToRow(at: IndexPath(row: 0, section: 2), at: .top, animated: false)
        table.layoutIfNeeded()
        let row = table.cellForRow(at: IndexPath(row: 0, section: 2))!.contentView.subviews.first as! UIStackView
        let type = row.arrangedSubviews.last as! UIButton
        (type.menu!.children[1] as! UIAction).performWithSender(nil, target: nil)
      }
      let rows = table.numberOfRows(inSection: 2)
      precondition(rows == (temporary ? 3 : 2))
      expectFailureScrolledIntoView(controller, screen: screen, error: error)
      let lastRow = table.rectForRow(at: IndexPath(row: rows - 1, section: 2)).height
      precondition(lastRow > table.estimatedRowHeight, "Rows must outgrow their estimate for this run to mean anything")
    }
    print("Create-channel accessibility passed at the largest text size: a failed create scrolls its whole error into view with two rows and with three")
  }

  @MainActor private static func expectFailureScrolledIntoView(_ controller: NativeCreateChannelViewController, screen: (CGSize, UIEdgeInsets), error: String) {
    let table = controller.tableView!
    table.frame.size = screen.0
    table.contentInset = screen.1
    table.layoutIfNeeded()
    table.contentOffset = CGPoint(x: 0, y: -table.adjustedContentInset.top)
    table.layoutIfNeeded()
    let visible = { table.bounds.inset(by: table.adjustedContentInset).insetBy(dx: 0, dy: -0.5) }
    let fold = visible().maxY
    precondition(table.contentSize.height > visible().height, "The form must overflow for this step to mean anything")
    controller.showError(error)
    table.layoutIfNeeded() // Measure the rows and materialize the views at the new offset.
    let footer = table.rectForFooter(inSection: 2)
    precondition(footer.minY > fold, "The error must start below the fold for this step to mean anything")
    precondition(visible().contains(footer), "A failed create must scroll its whole error into view")
    precondition(labels(in: table.footerView(forSection: 2)!).first!.text == error)
  }

  private static func labels(in view: UIView) -> [UILabel] {
    (view as? UILabel).map { [$0] } ?? view.subviews.flatMap(labels)
  }
}
