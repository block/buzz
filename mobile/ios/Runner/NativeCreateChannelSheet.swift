import Flutter
import UIKit

/// UIKit owns the complete form, including menus and keyboard presentation.
final class NativeCreateChannelSheetCoordinator: NSObject, UIAdaptivePresentationControllerDelegate {
  private let channel: FlutterMethodChannel
  private weak var parent: UIViewController?
  private var navigation: UINavigationController?
  private var pending: FlutterResult?

  init(messenger: FlutterBinaryMessenger, parentViewController: UIViewController?) {
    channel = FlutterMethodChannel(name: "buzz/create_channel_sheet", binaryMessenger: messenger)
    parent = parentViewController
    super.init()
    channel.setMethodCallHandler { [weak self] call, result in
      guard let self else { return }
      guard call.method == "present" else { result(FlutterMethodNotImplemented); return }
      guard self.navigation == nil, let data = call.arguments as? [String: Any],
        let session = data["session"] as? String else {
        result(FlutterError(code: "unavailable", message: "Cannot open New channel.", details: nil))
        return
      }
      var presenter = self.parent ?? UIApplication.shared.connectedScenes
        .compactMap { $0 as? UIWindowScene }.filter { $0.activationState == .foregroundActive }
        .flatMap(\.windows).first(where: \.isKeyWindow)?.rootViewController
      while let next = presenter?.presentedViewController { presenter = next }
      guard let presenter, presenter.view.window != nil, !presenter.isBeingDismissed else {
        result(FlutterError(code: "unavailable", message: "Cannot open New channel.", details: nil))
        return
      }
      let form = NativeCreateChannelViewController(configuration: data)
      form.onClose = { [weak self] in self?.close() }
      form.onCreate = { [weak self, weak form] values in
        self?.channel.invokeMethod("create", arguments: values.merging(["session": session]) { _, new in new }) { response in
          guard let self, let form else { return }
          if (response as? [String: Any])?["success"] as? Bool == true {
            self.close()
          } else {
            let error = (response as? [String: Any])?["error"] as? String ?? "Couldn't create channel. Please try again."
            form.showError(error)
          }
        }
      }
      let navigation = UINavigationController(rootViewController: form)
      navigation.overrideUserInterfaceStyle = data["dark"] as? Bool == true ? .dark : .light
      if UIDevice.current.userInterfaceIdiom == .pad { navigation.modalPresentationStyle = .formSheet }
      navigation.sheetPresentationController?.detents = [.large()]
      self.navigation = navigation
      self.pending = result
      presenter.present(navigation, animated: !UIAccessibility.isReduceMotionEnabled) {
        navigation.presentationController?.delegate = self
      }
    }
  }

  private func close() {
    guard let navigation else { return }
    navigation.dismiss(animated: !UIAccessibility.isReduceMotionEnabled) { [weak self] in self?.finish() }
  }

  private func finish() {
    navigation = nil
    let result = pending
    pending = nil
    result?(nil)
  }

  func presentationControllerDidDismiss(_ presentationController: UIPresentationController) { finish() }
}

final class NativeCreateChannelViewController: UITableViewController, UITextViewDelegate, UITextFieldDelegate {
  var onClose: (() -> Void)?
  var onCreate: (([String: Any]) -> Void)?
  private let name = UITextField()
  private let details = UITextView()
  private let detailsPlaceholder = UILabel()
  private let pageColor: UIColor
  private let rowColor: UIColor
  private var temporary = false
  private var visibility = "open"
  private var ttl = 604800
  private var busy = false
  private let expiry: [(String, Int)] = [
    ("30 minutes", 1800), ("1 hour", 3600), ("6 hours", 21600), ("12 hours", 43200),
    ("1 day", 86400), ("3 days", 259200), ("7 days", 604800), ("14 days", 1209600), ("30 days", 2592000),
  ]

  init(configuration: [String: Any]) {
    pageColor = Self.color(configuration["pageColor"]) ?? .systemGroupedBackground
    rowColor = Self.color(configuration["rowColor"]) ?? .secondarySystemGroupedBackground
    super.init(style: .insetGrouped)
    title = "New channel"
  }

  @available(*, unavailable)
  required init?(coder: NSCoder) { fatalError("init(coder:) has not been implemented") }

  override func viewDidLoad() {
    super.viewDidLoad()
    tableView.backgroundColor = pageColor
    tableView.keyboardDismissMode = .interactive
    tableView.rowHeight = UITableView.automaticDimension
    tableView.estimatedRowHeight = 56
    name.placeholder = "release-notes"
    name.accessibilityLabel = "Channel name"
    name.font = .preferredFont(forTextStyle: .body)
    name.adjustsFontForContentSizeCategory = true
    name.autocapitalizationType = .none
    name.autocorrectionType = .no
    name.spellCheckingType = .no
    name.returnKeyType = .next
    name.delegate = self
    name.addTarget(self, action: #selector(nameChanged), for: .editingChanged)
    details.font = .preferredFont(forTextStyle: .body)
    details.adjustsFontForContentSizeCategory = true
    details.backgroundColor = .clear
    details.accessibilityLabel = "Description, optional"
    details.delegate = self
    details.isScrollEnabled = true
    detailsPlaceholder.text = "What this channel is for"
    detailsPlaceholder.font = .preferredFont(forTextStyle: .body)
    detailsPlaceholder.adjustsFontForContentSizeCategory = true
    detailsPlaceholder.textColor = .placeholderText
    detailsPlaceholder.numberOfLines = 0
    detailsPlaceholder.isAccessibilityElement = false
    detailsPlaceholder.isUserInteractionEnabled = false
    detailsPlaceholder.translatesAutoresizingMaskIntoConstraints = false
    details.addSubview(detailsPlaceholder)
    NSLayoutConstraint.activate([
      detailsPlaceholder.leadingAnchor.constraint(equalTo: details.leadingAnchor, constant: details.textContainerInset.left + details.textContainer.lineFragmentPadding),
      detailsPlaceholder.trailingAnchor.constraint(equalTo: details.frameLayoutGuide.trailingAnchor, constant: -(details.textContainerInset.right + details.textContainer.lineFragmentPadding)),
      detailsPlaceholder.topAnchor.constraint(equalTo: details.topAnchor, constant: details.textContainerInset.top),
    ])
    navigationItem.leftBarButtonItem = UIBarButtonItem(image: UIImage(systemName: "xmark"), style: .plain, target: self, action: #selector(cancel))
    navigationItem.leftBarButtonItem?.accessibilityLabel = "Close"
    navigationItem.rightBarButtonItem = UIBarButtonItem(title: "Create", style: .done, target: self, action: #selector(create))
    NavigationTopEdge.configure(tableView, item: navigationItem)
    updateControls()
  }

  override func viewDidAppear(_ animated: Bool) {
    super.viewDidAppear(animated)
    name.becomeFirstResponder()
  }

  func textViewDidChange(_ textView: UITextView) { detailsPlaceholder.isHidden = !textView.text.isEmpty }

  @objc private func nameChanged() { updateControls() }
  func textFieldShouldReturn(_ textField: UITextField) -> Bool { details.becomeFirstResponder(); return false }

  private func updateControls() {
    navigationController?.isModalInPresentation = busy
    navigationItem.leftBarButtonItem?.isEnabled = !busy
    navigationItem.rightBarButtonItem?.isEnabled = !busy && !(name.text ?? "").trimmingCharacters(in: .whitespacesAndNewlines).isEmpty
    navigationItem.rightBarButtonItem?.title = busy ? "Creating…" : "Create"
    name.isEnabled = !busy
    details.isEditable = !busy
    for cell in tableView.visibleCells { cell.isUserInteractionEnabled = !busy }
  }

  @objc private func cancel() { if !busy { onClose?() } }
  @objc private func create() {
    let value = (name.text ?? "").trimmingCharacters(in: .whitespacesAndNewlines)
    guard !busy, !value.isEmpty else { return }
    busy = true
    tableView.tableFooterView = nil
    updateControls()
    var values: [String: Any] = ["name": value, "description": details.text.trimmingCharacters(in: .whitespacesAndNewlines), "visibility": visibility]
    if temporary { values["ttlSeconds"] = ttl }
    onCreate?(values)
  }

  func showError(_ error: String) {
    busy = false
    updateControls()
    let label = UILabel()
    label.text = error
    label.textColor = .systemRed
    label.font = .preferredFont(forTextStyle: .footnote)
    label.adjustsFontForContentSizeCategory = true
    label.numberOfLines = 0
    label.textAlignment = .center
    label.frame = CGRect(x: 20, y: 0, width: max(1, tableView.bounds.width - 40), height: 0)
    label.sizeToFit()
    tableView.tableFooterView = label
    UIAccessibility.post(notification: .announcement, argument: error)
  }

  override func numberOfSections(in tableView: UITableView) -> Int { 3 }
  override func tableView(_ tableView: UITableView, numberOfRowsInSection section: Int) -> Int { section == 2 ? (temporary ? 3 : 2) : 1 }
  override func tableView(_ tableView: UITableView, titleForHeaderInSection section: Int) -> String? { section == 0 ? "Name" : section == 1 ? "Description · Optional" : nil }

  override func tableView(_ tableView: UITableView, willDisplayHeaderView view: UIView, forSection section: Int) {
    // The editable field owns the label; its visual heading is decorative.
    guard section < 2 else { return }
    view.isAccessibilityElement = false
    view.accessibilityElementsHidden = true
    if let header = view as? UITableViewHeaderFooterView {
      header.textLabel?.isAccessibilityElement = false
      header.contentView.accessibilityElementsHidden = true
    }
  }

  override func tableView(_ tableView: UITableView, cellForRowAt indexPath: IndexPath) -> UITableViewCell {
    let cell = UITableViewCell(style: .default, reuseIdentifier: nil)
    cell.backgroundColor = rowColor
    cell.selectionStyle = .none
    cell.isUserInteractionEnabled = !busy
    if indexPath.section < 2 {
      let field: UIView = indexPath.section == 0 ? name : details
      field.removeFromSuperview()
      field.translatesAutoresizingMaskIntoConstraints = false
      cell.contentView.addSubview(field)
      NSLayoutConstraint.activate([
        field.leadingAnchor.constraint(equalTo: cell.contentView.layoutMarginsGuide.leadingAnchor),
        field.trailingAnchor.constraint(equalTo: cell.contentView.layoutMarginsGuide.trailingAnchor),
        field.topAnchor.constraint(equalTo: cell.contentView.topAnchor, constant: 12),
        field.bottomAnchor.constraint(equalTo: cell.contentView.bottomAnchor, constant: -12),
        field.heightAnchor.constraint(greaterThanOrEqualToConstant: indexPath.section == 0 ? 28 : 96),
      ])
    } else {
      let isType = indexPath.row == 0
      let isVisibility = indexPath.row == 1
      let label = isType ? "Channel type" : isVisibility ? "Visibility" : "Delete after inactivity"
      let value = isType ? (temporary ? "Temporary" : "Ongoing") : isVisibility ? (visibility == "open" ? "Public" : "Private") : expiry.first(where: { $0.1 == ttl })?.0 ?? "7 days"
      let button = UIButton(type: .system)
      let titleLabel = UILabel()
      titleLabel.text = label
      titleLabel.font = .preferredFont(forTextStyle: .body)
      titleLabel.adjustsFontForContentSizeCategory = true
      titleLabel.numberOfLines = 0
      titleLabel.isAccessibilityElement = false
      var configuration = UIButton.Configuration.plain()
      configuration.title = value
      configuration.baseForegroundColor = .secondaryLabel
      configuration.image = UIImage(systemName: "chevron.up.chevron.down")
      configuration.imagePlacement = .trailing
      configuration.imagePadding = 12
      configuration.contentInsets = NSDirectionalEdgeInsets(top: 14, leading: 8, bottom: 14, trailing: 16)
      button.configuration = configuration
      button.titleLabel?.adjustsFontForContentSizeCategory = true
      button.setContentHuggingPriority(.required, for: .horizontal)
      button.setContentCompressionResistancePriority(.required, for: .horizontal)
      button.isAccessibilityElement = true
      button.accessibilityLabel = label
      button.accessibilityValue = value
      button.showsMenuAsPrimaryAction = true
      let choices: [(String, String)] = isType ? [("Ongoing", "ongoing"), ("Temporary", "temporary")] : isVisibility ? [("Public", "open"), ("Private", "private")] : expiry.map { ($0.0, String($0.1)) }
      let selected = isType ? (temporary ? "temporary" : "ongoing") : isVisibility ? visibility : String(ttl)
      button.menu = UIMenu(children: choices.map { title, key in
        UIAction(title: title, state: key == selected ? .on : .off) { [weak self] _ in
          guard let self, !self.busy else { return }
          if isType { self.temporary = key == "temporary" }
          else if isVisibility { self.visibility = key }
          else if let seconds = Int(key) { self.ttl = seconds }
          self.tableView.reloadSections(IndexSet(integer: 2), with: .none)
        }
      })
      // Only the trailing value is lifted into UIKit's menu preview.
      let row = UIStackView(arrangedSubviews: [titleLabel, UIView(), button])
      row.spacing = 12
      row.alignment = .center
      row.translatesAutoresizingMaskIntoConstraints = false
      cell.contentView.addSubview(row)
      NSLayoutConstraint.activate([
        row.leadingAnchor.constraint(equalTo: cell.contentView.leadingAnchor, constant: 16),
        row.trailingAnchor.constraint(equalTo: cell.contentView.trailingAnchor),
        row.topAnchor.constraint(equalTo: cell.contentView.topAnchor),
        row.bottomAnchor.constraint(equalTo: cell.contentView.bottomAnchor),
        button.heightAnchor.constraint(greaterThanOrEqualToConstant: 48),
      ])
    }
    return cell
  }

  private static func color(_ value: Any?) -> UIColor? {
    guard let number = value as? NSNumber else { return nil }
    let argb = number.uint32Value
    return UIColor(red: CGFloat((argb >> 16) & 255) / 255, green: CGFloat((argb >> 8) & 255) / 255, blue: CGFloat(argb & 255) / 255, alpha: CGFloat((argb >> 24) & 255) / 255)
  }
}
