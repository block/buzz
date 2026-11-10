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
      if request == session { close(notify: false) { result(nil) } }
      else { result(nil) }
    default:
      result(FlutterMethodNotImplemented)
    }
  }

  private func close(notify: Bool, completion: (() -> Void)? = nil) {
    guard let request = session, let navigation else { completion?(); return }
    session = nil
    self.navigation = nil
    picker = nil
    navigation.dismiss(animated: !UIAccessibility.isReduceMotionEnabled) { [weak self] in
      if notify { self?.channel.invokeMethod("closed", arguments: ["session": request]) }
      completion?()
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
  /// Loaded avatars by key. Every state update rebuilds the cells, so a row
  /// draws its known avatar at once instead of blinking to its initial while
  /// the image comes back over the channel. Bounded like the Dart-side
  /// cache, so browsing a large community doesn't grow it without limit.
  private let avatars: NSCache<NSString, UIImage> = {
    let cache = NSCache<NSString, UIImage>()
    cache.countLimit = 64
    return cache
  }()
  private let search = UISearchController(searchResultsController: nil)
  private let pageColor: UIColor
  private let rowColor: UIColor
  /// The app theme's error color, which keeps 4.5:1 contrast on the sheet
  /// where `.systemRed` does not.
  private static let errorColor = UIColor { traits in
    traits.userInterfaceStyle == .dark
      ? UIColor(red: 0xFF / 255, green: 0x95 / 255, blue: 0x92 / 255, alpha: 1)
      : UIColor(red: 0xCE / 255, green: 0x2C / 255, blue: 0x31 / 255, alpha: 1)
  }
  private var selected: [[String: Any]] { state["selected"] as? [[String: Any]] ?? [] }
  private var users: [[String: Any]] {
    let selectedKeys = Set(selected.compactMap { $0["pubkey"] as? String })
    return (state["users"] as? [[String: Any]] ?? []).filter {
      !selectedKeys.contains($0["pubkey"] as? String ?? "")
    }
  }
  private var checkmarkAction: Bool { state["confirmWithCheckmark"] as? Bool == true }
  private var atLimit: Bool { selected.count >= (state["selectionLimit"] as? Int ?? Int.max) }
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
    title = state["title"] as? String ?? "Add members"
  }

  @available(*, unavailable)
  required init?(coder: NSCoder) { fatalError("init(coder:) has not been implemented") }

  override func viewDidLoad() {
    super.viewDidLoad()
    tableView.backgroundColor = pageColor
    tableView.sectionHeaderTopPadding = 0
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
    if checkmarkAction {
      navigationItem.leftBarButtonItem = UIBarButtonItem(image: UIImage(systemName: "xmark"), style: .plain, target: self, action: #selector(cancel))
      navigationItem.leftBarButtonItem?.accessibilityLabel = "Close"
      navigationItem.rightBarButtonItem = UIBarButtonItem(image: UIImage(systemName: "checkmark"), style: .done, target: self, action: #selector(add))
      navigationItem.rightBarButtonItem?.accessibilityLabel = "Create message"
    }
    NavigationTopEdge.configure(tableView, item: navigationItem)
    tableView.register(UITableViewCell.self, forCellReuseIdentifier: Self.cellIdentifier)
    rows = UITableViewDiffableDataSource(tableView: tableView) { [weak self] tableView, indexPath, item in
      let cell = tableView.dequeueReusableCell(withIdentifier: Self.cellIdentifier, for: indexPath)
      self?.configure(cell, item: item)
      return cell
    }
    rows.defaultRowAnimation = .fade
    applyState()
  }

  override func viewDidAppear(_ animated: Bool) {
    super.viewDidAppear(animated)
    // Activating in the same pass as the appearance is ignored, so focus
    // the field on the next turn of the run loop.
    DispatchQueue.main.async { [weak self] in
      guard let self, self.view.window != nil else { return }
      self.search.isActive = true
      self.search.searchBar.searchTextField.becomeFirstResponder()
    }
  }

  func update(_ data: [String: Any]) {
    // A queued render from before the latest keystroke must not replace results.
    guard (data["query"] as? String ?? "") == (search.searchBar.text ?? "") else { return }
    // Flutter sends the whole state on every rebuild, often unchanged.
    guard !NSDictionary(dictionary: data).isEqual(to: state) else { return }
    state = data
    if isViewLoaded { applyState() }
  }

  private func applyState() {
    navigationController?.isModalInPresentation = busy
    navigationItem.leftBarButtonItem?.isEnabled = !busy
    navigationItem.rightBarButtonItem?.isEnabled = !busy && !selected.isEmpty
    if checkmarkAction {
      navigationItem.rightBarButtonItem?.accessibilityLabel = busy ? "Creating message" : "Create message"
    } else {
      navigationItem.rightBarButtonItem?.title = busy ? "Adding…" : selected.isEmpty ? "Add" : "Add (\(selected.count))"
    }
    search.searchBar.isUserInteractionEnabled = !busy
    applySnapshot()
    let error = state["error"] as? String ?? ""
    if error.isEmpty {
      tableView.tableFooterView = nil
    } else {
      let label = UILabel()
      label.text = error
      label.textColor = Self.errorColor
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
    send("query", ["value": searchController.searchBar.text ?? ""])
  }

  @objc private func cancel() { if !busy { send("closed", [:]) } }
  @objc private func add() {
    guard !busy && !selected.isEmpty else { return }
    state["submitting"] = true
    applyState()
    send("submit", [:])
  }

  /// The status row after the results: the selection limit, a failed
  /// search, or why there are no results. Results stay shown while a search
  /// refreshes them.
  private var status: String? {
    if atLimit { return "DMs support up to nine people, including you." }
    if loadError { return "Couldn't search people or agents. Tap to retry." }
    if !users.isEmpty { return nil }
    if loading { return "Searching…" }
    if (state["query"] as? String ?? "").trimmingCharacters(in: .whitespaces).isEmpty,
      let hint = state["emptyQueryHint"] as? String {
      return hint
    }
    return "No matching people or agents."
  }
  private var shownUsers: [[String: Any]] { atLimit ? [] : users }

  /// A row: a member by pubkey, or the status row after the results. A
  /// member keeps its identity across sections, so a pick moves its row.
  private enum Item: Hashable {
    case member(String)
    case status
  }
  private static let cellIdentifier = "member"
  private var rows: UITableViewDiffableDataSource<Int, Item>!
  /// What each shown row was last drawn from, to redraw only changed rows.
  private var drawn: [Item: NSDictionary] = [:]

  private func member(_ pubkey: String) -> [String: Any]? {
    (selected + shownUsers).first { $0["pubkey"] as? String == pubkey }
  }

  private func drawing(_ item: Item) -> NSDictionary {
    switch item {
    case .member(let pubkey):
      return ["user": member(pubkey) ?? [:], "busy": busy, "selected": selected.contains { $0["pubkey"] as? String == pubkey }]
    case .status:
      return ["text": status ?? "", "retry": loadError && !atLimit, "busy": busy]
    }
  }

  /// Applies the state as a diff: unchanged rows keep their cells, changed
  /// ones are redrawn in place, and a pick or removal animates its row
  /// between the chosen and result sections.
  private func applySnapshot() {
    let chosen = selected.compactMap { $0["pubkey"] as? String }
    var results: [Item] = shownUsers.compactMap { ($0["pubkey"] as? String).map(Item.member) }
    if status != nil { results.append(.status) }
    var snapshot = NSDiffableDataSourceSnapshot<Int, Item>()
    snapshot.appendSections([0, 1])
    snapshot.appendItems(chosen.map(Item.member), toSection: 0)
    snapshot.appendItems(results, toSection: 1)
    let previous = rows.snapshot()
    let wasChosen = previous.numberOfSections > 0 ? previous.itemIdentifiers(inSection: 0) : []
    let next = Dictionary(uniqueKeysWithValues: snapshot.itemIdentifiers.map { ($0, drawing($0)) })
    let changed = snapshot.itemIdentifiers.filter { item in
      drawn[item].map { $0 != next[item] } ?? false
    }
    let shown = Set(previous.itemIdentifiers)
    snapshot.reconfigureItems(changed.filter(shown.contains))
    drawn = next
    let animate = view.window != nil && wasChosen != chosen.map(Item.member)
      && !UIAccessibility.isReduceMotionEnabled
    rows.apply(snapshot, animatingDifferences: animate)
  }

  // No section titles, and no grouped-table header bands: a small gap
  // above the first shown card and between the two.
  override func tableView(_ tableView: UITableView, heightForHeaderInSection section: Int) -> CGFloat {
    if section == 0 { return selected.isEmpty ? .leastNormalMagnitude : Self.cardGap }
    return selected.isEmpty ? Self.cardGap : Self.cardGap * 2
  }
  override func tableView(_ tableView: UITableView, heightForFooterInSection section: Int) -> CGFloat {
    .leastNormalMagnitude
  }
  override func tableView(_ tableView: UITableView, viewForHeaderInSection section: Int) -> UIView? { nil }
  override func tableView(_ tableView: UITableView, viewForFooterInSection section: Int) -> UIView? { nil }
  private static let cardGap: CGFloat = 8

  private func configure(_ cell: UITableViewCell, item: Item) {
    cell.backgroundColor = rowColor
    cell.accessoryType = .none
    cell.selectionStyle = .default
    cell.accessibilityTraits = .none
    cell.isAccessibilityElement = false
    cell.accessibilityLabel = nil
    cell.accessibilityIdentifier = nil
    cell.contentView.accessibilityElementsHidden = false
    cell.isUserInteractionEnabled = !busy
    var content = UIListContentConfiguration.subtitleCell()
    guard case .member(let pubkey) = item else {
      content.text = status
      content.textProperties.color = loadError && !atLimit ? Self.errorColor : .secondaryLabel
      content.textProperties.numberOfLines = 0
      cell.selectionStyle = loadError && !atLimit ? .default : .none
      if loadError && !atLimit { cell.accessibilityTraits.insert(.button) }
      cell.contentConfiguration = content
      return
    }
    guard let user = member(pubkey) else { preconditionFailure("A shown member row must be in the state") }
    let chosen = selected.contains { $0["pubkey"] as? String == pubkey }
    let key = user["avatarKey"] as? String
    content.text = user["name"] as? String
    content.secondaryText = user["detail"] as? String
    content.textProperties.numberOfLines = 0
    content.secondaryTextProperties.numberOfLines = 1
    content.image = key.flatMap { avatars.object(forKey: $0 as NSString) } ?? fallbackAvatar(user)
    content.imageProperties.maximumSize = CGSize(width: 40, height: 40)
    content.imageProperties.reservedLayoutSize = CGSize(width: 40, height: 40)
    cell.accessoryType = chosen ? .checkmark : .none
    if user["unavailable"] as? Bool == true {
      content.textProperties.color = .secondaryLabel
      cell.selectionStyle = .none
      cell.accessibilityTraits.insert(.notEnabled)
    }
    cell.accessibilityTraits.insert(.button)
    if chosen { cell.accessibilityTraits.insert(.selected) }
    cell.contentConfiguration = content
    cell.isAccessibilityElement = true
    cell.accessibilityLabel = [content.text, content.secondaryText]
      .compactMap { $0 }.filter { !$0.isEmpty }.joined(separator: ", ")
    cell.contentView.accessibilityElementsHidden = true
    guard let key else { return }
    cell.accessibilityIdentifier = key
    guard avatars.object(forKey: key as NSString) == nil else { return }
    loadAvatar(pubkey, key) { [weak self, weak cell] data in
      guard let data, let image = UIImage(data: data) else { return }
      let avatar = image.withRenderingMode(.alwaysOriginal)
      self?.avatars.setObject(avatar, forKey: key as NSString)
      guard let cell, cell.accessibilityIdentifier == key,
        var configuration = cell.contentConfiguration as? UIListContentConfiguration else { return }
      configuration.image = avatar
      let label = cell.accessibilityLabel
      cell.contentConfiguration = configuration
      cell.accessibilityLabel = label
      cell.contentView.accessibilityElementsHidden = true
    }
  }

  override func tableView(_ tableView: UITableView, didSelectRowAt indexPath: IndexPath) {
    tableView.deselectRow(at: indexPath, animated: true)
    guard !busy, let item = rows.itemIdentifier(for: indexPath) else { return }
    guard case .member(let pubkey) = item else {
      if loadError && !atLimit { send("retry", [:]) }
      return
    }
    guard let user = member(pubkey) else { preconditionFailure("A shown member row must be in the state") }
    if user["unavailable"] as? Bool == true { return }
    send("toggle", ["pubkey": pubkey])
    // Where asked (New DM), a pick ends its query, as an inserted mention
    // does, so the next name starts fresh.
    if indexPath.section == 1 && state["clearQueryOnPick"] as? Bool == true
      && !(search.searchBar.text ?? "").isEmpty {
      search.searchBar.text = ""
      state["query"] = ""
      send("query", ["value": ""])
    }
  }

  private func fallbackAvatar(_ user: [String: Any]) -> UIImage {
    let size = CGSize(width: 40, height: 40)
    return UIGraphicsImageRenderer(size: size).image { _ in
      (Self.color(user["avatarBackground"]) ?? .tertiarySystemFill).setFill()
      let rect = CGRect(origin: .zero, size: size)
      (user["agent"] as? Bool == true ? Self.agentSquircle(rect) : UIBezierPath(ovalIn: rect)).fill()
      let text = (user["initial"] as? String ?? "?") as NSString
      let attributes: [NSAttributedString.Key: Any] = [
        .font: UIFont.systemFont(ofSize: 18, weight: .medium),
        .foregroundColor: Self.color(user["avatarForeground"]) ?? UIColor.label,
      ]
      let measured = text.size(withAttributes: attributes)
      text.draw(at: CGPoint(x: (40 - measured.width) / 2, y: (40 - measured.height) / 2), withAttributes: attributes)
    }.withRenderingMode(.alwaysOriginal)
  }

  /// Mirrors avatar_shape.dart and desktop's `ROUNDED_SQUIRCLE_PATH`
  /// (desktop/src/shared/ui/AvatarClipPaths.tsx).
  private static func agentSquircle(_ rect: CGRect) -> UIBezierPath {
    func at(_ x: CGFloat, _ y: CGFloat) -> CGPoint {
      CGPoint(x: rect.minX + rect.width * x, y: rect.minY + rect.height * y)
    }
    let path = UIBezierPath()
    path.move(to: at(0.5, 0))
    path.addCurve(to: at(1, 0.5), controlPoint1: at(0.93, 0), controlPoint2: at(1, 0.07))
    path.addCurve(to: at(0.5, 1), controlPoint1: at(1, 0.93), controlPoint2: at(0.93, 1))
    path.addCurve(to: at(0, 0.5), controlPoint1: at(0.07, 1), controlPoint2: at(0, 0.93))
    path.addCurve(to: at(0.5, 0), controlPoint1: at(0, 0.07), controlPoint2: at(0.07, 0))
    path.close()
    return path
  }

  private static func color(_ value: Any?) -> UIColor? {
    guard let number = value as? NSNumber else { return nil }
    let argb = number.uint32Value
    return UIColor(red: CGFloat((argb >> 16) & 255) / 255, green: CGFloat((argb >> 8) & 255) / 255,
      blue: CGFloat(argb & 255) / 255, alpha: CGFloat((argb >> 24) & 255) / 255)
  }
}
