import Flutter
import UIKit

/// One interaction on the Flutter host. Flutter identifies the touched row
/// and sends its bounds only when a pointer goes down.
final class ConversationMenu: NSObject, UIContextMenuInteractionDelegate {
  private let host: () -> UIViewController?
  private let bridge: FlutterMethodChannel
  private lazy var interaction = UIContextMenuInteraction(delegate: self)
  private var touchId = 0
  private var armedRect: CGRect?
  private var snapshot: UIView?
  private var updatedLabels: [String]?
  private var selection: String?
  private var active = false

  init(messenger: FlutterBinaryMessenger, host: @escaping () -> UIViewController?) {
    self.host = host
    bridge = FlutterMethodChannel(name: "buzz/conversation_menu", binaryMessenger: messenger)
    super.init()
    bridge.setMethodCallHandler { [weak self] call, result in
      guard let self else { result(nil); return }
      switch call.method {
      case "attach", "arm":
        if let view = self.host()?.view, self.interaction.view !== view {
          self.interaction.view?.removeInteraction(self.interaction)
          view.addInteraction(self.interaction)
        }
        if let bounds = call.arguments as? [CGFloat], bounds.count == 5 {
          self.touchId = Int(bounds[4])
          self.armedRect = CGRect(x: bounds[0], y: bounds[1], width: bounds[2], height: bounds[3])
        }
      case "disarm": self.armedRect = nil
      case "dismiss": self.interaction.dismissMenu()
      case "update":
        if self.active, let labels = call.arguments as? [String] {
          self.updatedLabels = labels
          self.interaction.updateVisibleMenu { _ in UIMenu(children: self.actions(labels)) }
        }
      default: result(FlutterMethodNotImplemented); return
      }
      result(nil)
    }
  }

  private func actions(_ labels: [String]) -> [UIAction] {
    let icons = [
      "Rename": "tabler-pencil", "Archive": "archive", "Unarchive": "archive",
      "Mark read": "mail", "Mark unread": "mail",
      "Change workspace": "folder", "Move to section…": "folder-input",
    ]
    return labels.map { label in
      UIAction(title: label, image: icons[label].flatMap { NativeTabler.image($0, size: 24) }) {
        [weak self] _ in
        self?.selection = label
      }
    }
  }

  func contextMenuInteraction(
    _ interaction: UIContextMenuInteraction, configurationForMenuAtLocation location: CGPoint
  ) -> UIContextMenuConfiguration? {
    guard !active, let view = interaction.view, let window = view.window,
      let rect = armedRect, rect.contains(location),
      let copy = window.resizableSnapshotView(from: view.convert(rect, to: window),
        afterScreenUpdates: false, withCapInsets: .zero) else { return nil }
    active = true
    selection = nil
    updatedLabels = nil
    copy.frame = rect
    copy.isUserInteractionEnabled = false
    view.addSubview(copy)
    snapshot = copy
    let requestId = touchId
    return UIContextMenuConfiguration(identifier: nil, previewProvider: nil) { [weak self] _ in
      let deferred = UIDeferredMenuElement.uncached { [weak self] completion in
        guard let self, self.active else {
          completion([])
          return
        }
        self.bridge.invokeMethod("prepare", arguments: requestId) { [weak self] result in
          guard let self, self.active, let labels = result as? [String] else {
            completion([])
            return
          }
          completion(self.actions(self.updatedLabels ?? labels))
        }
      }
      return UIMenu(children: [deferred])
    }
  }

  func contextMenuInteraction(_ interaction: UIContextMenuInteraction,
    previewForHighlightingMenuWithConfiguration configuration: UIContextMenuConfiguration
  ) -> UITargetedPreview? {
    snapshot.map { UITargetedPreview(view: $0) }
  }

  func contextMenuInteraction(_ interaction: UIContextMenuInteraction,
    previewForDismissingMenuWithConfiguration configuration: UIContextMenuConfiguration
  ) -> UITargetedPreview? {
    snapshot.map { UITargetedPreview(view: $0) }
  }

  func contextMenuInteraction(
    _ interaction: UIContextMenuInteraction, willDisplayMenuFor configuration: UIContextMenuConfiguration,
    animator: UIContextMenuInteractionAnimating?
  ) {
    animator?.addCompletion { [weak self] in
      guard let self, self.active, let labels = self.updatedLabels else { return }
      interaction.updateVisibleMenu { _ in UIMenu(children: self.actions(labels)) }
    }
  }

  func contextMenuInteraction(
    _ interaction: UIContextMenuInteraction, willEndFor configuration: UIContextMenuConfiguration,
    animator: UIContextMenuInteractionAnimating?
  ) {
    let finish = { [weak self] in
      guard let self else { return }
      self.snapshot?.removeFromSuperview()
      self.snapshot = nil
      self.active = false
      self.bridge.invokeMethod("end", arguments: self.selection)
    }
    if let animator { animator.addCompletion(finish) } else { finish() }
  }
}
