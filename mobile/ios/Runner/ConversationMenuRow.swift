import Flutter
import UIKit

final class ConversationMenuRowFactory: NSObject, FlutterPlatformViewFactory {
  private let messenger: FlutterBinaryMessenger
  init(_ messenger: FlutterBinaryMessenger) { self.messenger = messenger }

  func create(withFrame frame: CGRect, viewIdentifier id: Int64, arguments: Any?)
    -> FlutterPlatformView
  {
    ConversationMenuRow(frame: frame, id: id, messenger: messenger)
  }
}

/// The Flutter row supplies its appearance and actions. Its transparent native
/// hit target lets UIKit own the original hold gesture, preview and dismissal.
private final class ConversationMenuRow: NSObject, FlutterPlatformView,
  UIContextMenuInteractionDelegate
{
  private let hitView: UIView
  private let bridge: FlutterMethodChannel
  private lazy var interaction = UIContextMenuInteraction(delegate: self)
  private var snapshot: UIView?
  private var updatedLabels: [String]?
  private var selection: String?
  private var active = false

  init(frame: CGRect, id: Int64, messenger: FlutterBinaryMessenger) {
    hitView = UIView(frame: frame)
    bridge = FlutterMethodChannel(name: "buzz/conversation_row/\(id)", binaryMessenger: messenger)
    super.init()
    hitView.addInteraction(interaction)
    bridge.setMethodCallHandler { [weak self] call, result in
      switch call.method {
      case "dismiss": self?.interaction.dismissMenu()
      case "update":
        if let self, self.active, let labels = call.arguments as? [String] {
          self.updatedLabels = labels
          self.interaction.updateVisibleMenu { _ in UIMenu(children: self.actions(labels)) }
        }
      default:
        result(FlutterMethodNotImplemented)
        return
      }
      result(nil)
    }
    hitView.addGestureRecognizer(UITapGestureRecognizer(target: self, action: #selector(tapped)))
  }
  func view() -> UIView { hitView }
  @objc private func tapped() { if !active { bridge.invokeMethod("tap", arguments: nil) } }

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
    guard !active, let window = hitView.window else { return nil }
    active = true
    selection = nil
    updatedLabels = nil
    let rect = hitView.convert(hitView.bounds, to: window)
    if let copy = window.resizableSnapshotView(
      from: rect, afterScreenUpdates: false, withCapInsets: .zero)
    {
      copy.frame = hitView.bounds
      copy.isUserInteractionEnabled = false
      hitView.addSubview(copy)
      snapshot = copy
    }
    return UIContextMenuConfiguration(identifier: nil, previewProvider: nil) { [weak self] _ in
      let deferred = UIDeferredMenuElement.uncached { [weak self] completion in
        guard let self, self.active else {
          completion([])
          return
        }
        self.bridge.invokeMethod("prepare", arguments: nil) { [weak self] result in
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
