import Flutter
import UIKit

/// UIKit owns presentation; Flutter retains search, selection and relay writes.
final class NativeAddMembersSheetCoordinator: NSObject, UIAdaptivePresentationControllerDelegate {
  private let channel: FlutterMethodChannel
  private weak var parent: UIViewController?
  private var navigation: UINavigationController?
  private var picker: NativeAddMembersViewController?
  private var session: String?

  init(messenger: FlutterBinaryMessenger, parentViewController: UIViewController?) {
    channel = FlutterMethodChannel(name: "buzz/add_members_sheet", binaryMessenger: messenger)
    parent = parentViewController
    super.init()
    channel.setMethodCallHandler { [weak self] call, result in
      self?.handle(call, result: result)
    }
  }

  private func handle(_ call: FlutterMethodCall, result: @escaping FlutterResult) {
    guard let data = call.arguments as? [String: Any], let request = data["session"] as? String else {
      result(FlutterError(code: "invalid_arguments", message: "Expected a picker session.", details: nil))
      return
    }
    switch call.method {
    case "present":
      guard navigation == nil else {
        result(FlutterError(code: "busy", message: "A member picker is already open.", details: nil))
        return
      }
      var presenter = parent ?? UIApplication.shared.connectedScenes
        .compactMap { $0 as? UIWindowScene }
        .filter { $0.activationState == .foregroundActive }
        .flatMap(\.windows).first(where: \.isKeyWindow)?.rootViewController
      while let next = presenter?.presentedViewController { presenter = next }
      guard let presenter, presenter.view.window != nil, !presenter.isBeingDismissed else {
        result(FlutterError(code: "unavailable", message: "Cannot open the member picker.", details: nil))
        return
      }
      session = request
      let picker = NativeAddMembersViewController(state: data, loadAvatar: { [weak self] pubkey, key, completion in
        guard let self, self.session == request else { completion(nil); return }
        self.channel.invokeMethod("avatar", arguments: ["session": request, "pubkey": pubkey, "avatarKey": key]) { value in
          completion((value as? FlutterStandardTypedData)?.data)
        }
      }) { [weak self] event, values in
        guard let self, self.session == request else { return }
        if event == "closed" {
          self.close(notify: true)
        } else {
          self.channel.invokeMethod(event, arguments: ["session": request].merging(values) { _, new in new })
        }
      }
      let navigation = UINavigationController(rootViewController: picker)
      navigation.overrideUserInterfaceStyle = data["dark"] as? Bool == true ? .dark : .light
      if UIDevice.current.userInterfaceIdiom == .pad { navigation.modalPresentationStyle = .formSheet }
      navigation.sheetPresentationController?.detents = [.large()]
      self.navigation = navigation
      self.picker = picker
      presenter.present(navigation, animated: !UIAccessibility.isReduceMotionEnabled) {
        navigation.presentationController?.delegate = self
        result(nil)
      }
    case "update":
      if request == session { picker?.update(data) }
      result(nil)
    case "dismiss":
      if request == session { close(notify: false) }
      result(nil)
    default:
      result(FlutterMethodNotImplemented)
    }
  }

  private func close(notify: Bool) {
    guard let request = session, let navigation else { return }
    session = nil
    self.navigation = nil
    picker = nil
    navigation.dismiss(animated: !UIAccessibility.isReduceMotionEnabled) { [weak self] in
      if notify { self?.channel.invokeMethod("closed", arguments: ["session": request]) }
    }
  }

  func presentationControllerDidDismiss(_ presentationController: UIPresentationController) {
    guard let request = session else { return }
    session = nil
    navigation = nil
    picker = nil
    channel.invokeMethod("closed", arguments: ["session": request])
  }
}

final class NativeAddMembersViewController: UITableViewController, UISearchResultsUpdating {
  private var state: [String: Any]
  private let send: (String, [String: Any]) -> Void
  private let loadAvatar: (String, String, @escaping (Data?) -> Void) -> Void
  private let search = UISearchController(searchResultsController: nil)
  private let pageColor: UIColor
  private let rowColor: UIColor
  private var selected: [[String: Any]] { state["selected"] as? [[String: Any]] ?? [] }
  private var users: [[String: Any]] {
    let selectedKeys = Set(selected.compactMap { $0["pubkey"] as? String })
    return (state["users"] as? [[String: Any]] ?? []).filter {
      !selectedKeys.contains($0["pubkey"] as? String ?? "")
    }
  }
  private var busy: Bool { state["submitting"] as? Bool == true }
  private var loading: Bool { state["loading"] as? Bool == true }
  private var loadError: Bool { state["loadError"] as? Bool == true }

  init(state: [String: Any], loadAvatar: @escaping (String, String, @escaping (Data?) -> Void) -> Void, send: @escaping (String, [String: Any]) -> Void) {
    self.state = state
    self.loadAvatar = loadAvatar
    self.send = send
    pageColor = Self.color(state["pageColor"]) ?? .systemGroupedBackground
    rowColor = Self.color(state["rowColor"]) ?? .secondarySystemGroupedBackground
    super.init(style: .insetGrouped)
    title = "Add members"
  }

  @available(*, unavailable)
  required init?(coder: NSCoder) { fatalError("init(coder:) has not been implemented") }

  override func viewDidLoad() {
    super.viewDidLoad()
    tableView.backgroundColor = pageColor
    tableView.keyboardDismissMode = .interactive
    tableView.rowHeight = UITableView.automaticDimension
    tableView.estimatedRowHeight = 64
    search.obscuresBackgroundDuringPresentation = false
    search.hidesNavigationBarDuringPresentation = false
    search.automaticallyShowsCancelButton = false
    search.searchResultsUpdater = self
    search.searchBar.placeholder = "Search for people or agents"
    search.searchBar.autocorrectionType = .no
    search.searchBar.autocapitalizationType = .none
    search.searchBar.showsCancelButton = false
    navigationItem.searchController = search
    navigationItem.hidesSearchBarWhenScrolling = false
    definesPresentationContext = true
    navigationItem.leftBarButtonItem = UIBarButtonItem(barButtonSystemItem: .cancel, target: self, action: #selector(cancel))
    navigationItem.rightBarButtonItem = UIBarButtonItem(title: "Add", style: .done, target: self, action: #selector(add))
    let appearance = UINavigationBarAppearance()
    appearance.configureWithOpaqueBackground()
    appearance.backgroundColor = pageColor
    appearance.shadowColor = .clear
    navigationItem.standardAppearance = appearance
    navigationItem.scrollEdgeAppearance = appearance
    applyState()
  }

  override func viewDidAppear(_ animated: Bool) {
    super.viewDidAppear(animated)
    search.isActive = true
    search.searchBar.becomeFirstResponder()
  }

  func update(_ data: [String: Any]) {
    // A queued render from before the latest keystroke must not replace results.
    guard (data["query"] as? String ?? "") == (search.searchBar.text ?? "") else { return }
    state = data
    if isViewLoaded { applyState() }
  }

  private func applyState() {
    navigationController?.isModalInPresentation = busy
    navigationItem.leftBarButtonItem?.isEnabled = !busy
    navigationItem.rightBarButtonItem?.isEnabled = !busy && !selected.isEmpty
    navigationItem.rightBarButtonItem?.title = busy ? "Adding…" : selected.isEmpty ? "Add" : "Add (\(selected.count))"
    search.searchBar.isUserInteractionEnabled = !busy
    tableView.reloadData()
    let error = state["error"] as? String ?? ""
    if error.isEmpty {
      tableView.tableFooterView = nil
    } else {
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
  }

  func updateSearchResults(for searchController: UISearchController) {
    guard !busy else { return }
    let query = searchController.searchBar.text ?? ""
    guard query != (state["query"] as? String ?? "") else { return }
    state["query"] = query
    state["loading"] = true
    tableView.reloadData()
    send("query", ["value": searchController.searchBar.text ?? ""])
  }

  @objc private func cancel() { if !busy { send("closed", [:]) } }
  @objc private func add() {
    guard !busy && !selected.isEmpty else { return }
    state["submitting"] = true
    applyState()
    send("submit", [:])
  }

  override func numberOfSections(in tableView: UITableView) -> Int { 2 }
  override func tableView(_ tableView: UITableView, numberOfRowsInSection section: Int) -> Int {
    section == 0 ? selected.count : (loading || loadError || users.isEmpty ? 1 : users.count)
  }
  override func tableView(_ tableView: UITableView, titleForHeaderInSection section: Int) -> String? {
    section == 0 ? (selected.isEmpty ? nil : "Selected") : "People and agents"
  }
  override func tableView(_ tableView: UITableView, cellForRowAt indexPath: IndexPath) -> UITableViewCell {
    let cell = UITableViewCell(style: .subtitle, reuseIdentifier: nil)
    cell.backgroundColor = rowColor
    var content = cell.defaultContentConfiguration()
    var avatarRequest: (String, String)?
    var memberLabel: String?
    if indexPath.section == 1 && (loading || loadError || users.isEmpty) {
      content.text = loading ? "Searching…" : loadError ? "Couldn't load people or agents. Tap to retry." : "No matching people or agents."
      content.textProperties.color = .secondaryLabel
      content.textProperties.numberOfLines = 0
      cell.selectionStyle = loadError && !loading ? .default : .none
      if loadError && !loading { cell.accessibilityTraits.insert(.button) }
    } else {
      let user = indexPath.section == 0 ? selected[indexPath.row] : users[indexPath.row]
      content.text = user["name"] as? String
      content.secondaryText = user["detail"] as? String
      memberLabel = [content.text, content.secondaryText]
        .compactMap { $0 }.filter { !$0.isEmpty }.joined(separator: ", ")
      content.textProperties.numberOfLines = 0
      content.secondaryTextProperties.numberOfLines = 1
      content.image = fallbackAvatar(user)
      content.imageProperties.maximumSize = CGSize(width: 40, height: 40)
      content.imageProperties.reservedLayoutSize = CGSize(width: 40, height: 40)
      if let pubkey = user["pubkey"] as? String, let key = user["avatarKey"] as? String {
        avatarRequest = (pubkey, key)
      }
      cell.accessoryType = indexPath.section == 0 ? .checkmark : .none
      cell.accessibilityTraits.insert(.button)
      if indexPath.section == 0 { cell.accessibilityTraits.insert(.selected) }
    }
    cell.contentConfiguration = content
    if let memberLabel {
      cell.isAccessibilityElement = true
      cell.accessibilityLabel = memberLabel
      cell.contentView.accessibilityElementsHidden = true
    }
    if let (pubkey, key) = avatarRequest {
      cell.accessibilityIdentifier = key
      loadAvatar(pubkey, key) { [weak cell] data in
        guard let cell, cell.accessibilityIdentifier == key,
          let data, let image = UIImage(data: data),
          var configuration = cell.contentConfiguration as? UIListContentConfiguration else { return }
        configuration.image = image.withRenderingMode(.alwaysOriginal)
        let label = cell.accessibilityLabel
        cell.contentConfiguration = configuration
        cell.accessibilityLabel = label
        cell.contentView.accessibilityElementsHidden = true
      }
    }
    cell.isUserInteractionEnabled = !busy
    return cell
  }
  override func tableView(_ tableView: UITableView, didSelectRowAt indexPath: IndexPath) {
    tableView.deselectRow(at: indexPath, animated: true)
    guard !busy else { return }
    if indexPath.section == 1 && (loading || loadError || users.isEmpty) {
      if loadError && !loading { send("retry", [:]) }
      return
    }
    let user = indexPath.section == 0 ? selected[indexPath.row] : users[indexPath.row]
    if let pubkey = user["pubkey"] as? String { send("toggle", ["pubkey": pubkey]) }
  }

  private func fallbackAvatar(_ user: [String: Any]) -> UIImage {
    let size = CGSize(width: 40, height: 40)
    return UIGraphicsImageRenderer(size: size).image { _ in
      (Self.color(user["avatarBackground"]) ?? .tertiarySystemFill).setFill()
      UIBezierPath(roundedRect: CGRect(origin: .zero, size: size),
        cornerRadius: user["agent"] as? Bool == true ? 12 : 20).fill()
      let text = (user["initial"] as? String ?? "?") as NSString
      let attributes: [NSAttributedString.Key: Any] = [
        .font: UIFont.systemFont(ofSize: 18, weight: .medium),
        .foregroundColor: Self.color(user["avatarForeground"]) ?? UIColor.label,
      ]
      let measured = text.size(withAttributes: attributes)
      text.draw(at: CGPoint(x: (40 - measured.width) / 2, y: (40 - measured.height) / 2), withAttributes: attributes)
    }.withRenderingMode(.alwaysOriginal)
  }

  private static func color(_ value: Any?) -> UIColor? {
    guard let number = value as? NSNumber else { return nil }
    let argb = number.uint32Value
    return UIColor(red: CGFloat((argb >> 16) & 255) / 255, green: CGFloat((argb >> 8) & 255) / 255,
      blue: CGFloat(argb & 255) / 255, alpha: CGFloat((argb >> 24) & 255) / 255)
  }
}
