import UIKit

// UIKit 9127.0.85 is the iOS 27.1 SDK with Duo APIs. SDK checks must
// surround runtime availability checks: older SDKs cannot resolve these symbols.

/// Status uses the same navigation-controller sheet as the profile text forms.
final class NativeStatusEditorViewController: UITableViewController, UITextFieldDelegate {
  let statusField = UITextField()
  let emojiField = StatusEmojiTextField()
  let datePicker = UIDatePicker()
  private let configuration: [String: Any]
  private let onChooseEmoji: (@escaping (String?) -> Void) -> Void
  private let onCancel: () -> Void
  private let onSet: ([String: Any]) -> Void
  private var duration: Int
  private let durations = [3600, 28800, 86400, 604800]
  private let durationLabels = ["1 hour", "8 hours", "1 day", "1 week", "Custom"]
  private let presets = [("🗣️", "In a meeting"), ("🚌", "Commuting"),
                         ("🤒", "Out sick"), ("🏖️", "Vacationing"), ("🏠", "Working remotely")]

  init(configuration: [String: Any], onChooseEmoji: @escaping (@escaping (String?) -> Void) -> Void = { $0(nil) }, onCancel: @escaping () -> Void,
       onSet: @escaping ([String: Any]) -> Void) {
    self.configuration = configuration
    self.onChooseEmoji = onChooseEmoji
    self.onCancel = onCancel
    self.onSet = onSet
    duration = configuration["duration"] as? Int ?? 86400
    super.init(style: .insetGrouped)
    title = "Set a status"
  }

  required init?(coder: NSCoder) { fatalError("init(coder:) has not been implemented") }

  override func viewDidLoad() {
    super.viewDidLoad()
    view.tintColor = .label
    navigationController?.view.tintColor = .label
    navigationController?.navigationBar.tintColor = .label
    let pageColor = color("pageBackgroundArgb", fallback: .systemGroupedBackground)
    tableView.backgroundColor = pageColor
    // Space the form below the bar without moving UIKit’s native rail actions.
    tableView.tableHeaderView = UIView(frame: CGRect(x: 0, y: 0, width: 1, height: 16))
    let appearance = UINavigationBarAppearance()
    appearance.configureWithOpaqueBackground()
    appearance.backgroundColor = pageColor
    navigationItem.standardAppearance = appearance
    navigationItem.scrollEdgeAppearance = appearance
    navigationItem.compactAppearance = appearance
    navigationItem.leftBarButtonItem = UIBarButtonItem(
      barButtonSystemItem: .close, target: self, action: #selector(cancel))
    navigationItem.leftBarButtonItem?.accessibilityLabel = "Close"
    navigationItem.rightBarButtonItem = UIBarButtonItem(
      title: "Set", style: .done, target: self, action: #selector(save))
    #if canImport(UIKit, _version: 9127.0.85)
    if #available(iOS 27.1, *) {
      navigationItem.leftBarButtonItem?.axisBehavior = .verticalPreferred
      navigationItem.rightBarButtonItem?.axisBehavior = .verticalPreferred
    }
    #endif
    statusField.text = configuration["text"] as? String ?? ""
    statusField.placeholder = "What’s your status?"
    statusField.accessibilityLabel = "Status"
    statusField.returnKeyType = .done
    statusField.delegate = self
    emojiField.text = configuration["emoji"] as? String ?? ""
    emojiField.placeholder = "Emoji"
    emojiField.accessibilityLabel = "Status emoji"
    emojiField.autocorrectionType = .no
    emojiField.autocapitalizationType = .none
    emojiField.clearButtonMode = .whileEditing
    for field in [statusField, emojiField] {
      field.font = .preferredFont(forTextStyle: .body)
      field.adjustsFontForContentSizeCategory = true
      field.addTarget(self, action: #selector(changed), for: .editingChanged)
    }
    datePicker.tintColor = .label
    datePicker.datePickerMode = .dateAndTime
    datePicker.preferredDatePickerStyle = .compact
    datePicker.minimumDate = Date().addingTimeInterval(300)
    let expiration = configuration["expiresAt"] as? Double ?? Date().addingTimeInterval(86400).timeIntervalSince1970
    datePicker.date = max(Date(timeIntervalSince1970: expiration), datePicker.minimumDate ?? Date())
    datePicker.accessibilityLabel = "Custom expiration"
    tableView.keyboardDismissMode = .interactive
    changed()
  }

  override func viewDidAppear(_ animated: Bool) {
    super.viewDidAppear(animated)
    if configuration["saveError"] as? Bool == true, !didShowError {
      didShowError = true
      let alert = UIAlertController(title: "Couldn’t save status", message: "Your changes are still here. Please try again.", preferredStyle: .alert)
      alert.view.tintColor = .label
      alert.addAction(UIAlertAction(title: "OK", style: .default))
      present(alert, animated: true)
    }
  }

  private var didShowError = false

  override func numberOfSections(in tableView: UITableView) -> Int {
    configuration["hasExistingStatus"] as? Bool == true ? 4 : 3
  }

  override func tableView(_ tableView: UITableView, numberOfRowsInSection section: Int) -> Int {
    switch section {
    case 0: return 2
    case 1: return duration == 0 ? 2 : 1
    case 2: return presets.count
    default: return 1
    }
  }

  override func tableView(_ tableView: UITableView, titleForHeaderInSection section: Int) -> String? {
    section == 1 ? "Duration" : section == 2 ? "Quick statuses" : nil
  }

  private func color(_ key: String, fallback: UIColor) -> UIColor {
    guard let value = configuration[key] as? NSNumber else { return fallback }
    let argb = value.uint32Value
    return UIColor(red: CGFloat((argb >> 16) & 255) / 255,
                   green: CGFloat((argb >> 8) & 255) / 255,
                   blue: CGFloat(argb & 255) / 255,
                   alpha: CGFloat((argb >> 24) & 255) / 255)
  }

  private func embed(_ content: UIView, in cell: UITableViewCell) {
    content.translatesAutoresizingMaskIntoConstraints = false
    cell.contentView.addSubview(content)
    NSLayoutConstraint.activate([
      content.leadingAnchor.constraint(equalTo: cell.contentView.layoutMarginsGuide.leadingAnchor),
      content.trailingAnchor.constraint(equalTo: cell.contentView.layoutMarginsGuide.trailingAnchor),
      content.topAnchor.constraint(equalTo: cell.contentView.topAnchor, constant: 12),
      content.bottomAnchor.constraint(equalTo: cell.contentView.bottomAnchor, constant: -12),
    ])
  }

  override func tableView(_ tableView: UITableView, cellForRowAt indexPath: IndexPath) -> UITableViewCell {
    let cell = UITableViewCell(style: .value1, reuseIdentifier: nil)
    cell.backgroundColor = color("containerBackgroundArgb", fallback: .secondarySystemGroupedBackground)
    cell.selectionStyle = .none
    switch indexPath.section {
    case 0:
      if indexPath.row == 0 {
        embed(statusField, in: cell)
      } else {
        let choose = UIButton(type: .system)
        choose.setTitle("Choose emoji", for: .normal)
        choose.addAction(UIAction { [weak self, weak choose] _ in
          guard let self else { return }
          self.view.endEditing(true)
          choose?.isEnabled = false
          choose?.setTitle("Loading…", for: .normal)
          self.onChooseEmoji { [weak self, weak choose] value in
            choose?.isEnabled = true
            choose?.setTitle("Choose emoji", for: .normal)
            guard let self, let value else { return }
            self.emojiField.text = value
            self.changed()
          }
        }, for: .touchUpInside)
        let row = UIStackView(arrangedSubviews: [emojiField, choose])
        row.spacing = 12
        choose.setContentHuggingPriority(.required, for: .horizontal)
        embed(row, in: cell)
      }
    case 1:
      if indexPath.row == 1 {
        embed(datePicker, in: cell)
      } else {
        cell.textLabel?.text = "Clear after"
        let button = UIButton(type: .system)
        let selected = durations.firstIndex(of: duration) ?? 4
        button.setTitle(durationLabels[selected], for: .normal)
        button.accessibilityLabel = "Duration"
        button.showsMenuAsPrimaryAction = true
        button.menu = UIMenu(children: durationLabels.enumerated().map { index, label in
          UIAction(title: label, state: index == selected ? .on : .off) { [weak self] _ in
            guard let self else { return }
            self.duration = index < self.durations.count ? self.durations[index] : 0
            self.tableView.reloadSections(IndexSet(integer: 1), with: .automatic)
          }
        })
        button.sizeToFit()
        cell.accessoryView = button
      }
    case 2:
      cell.textLabel?.text = "\(presets[indexPath.row].0)  \(presets[indexPath.row].1)"
      cell.accessibilityTraits.insert(.button)
      cell.selectionStyle = .default
    default:
      cell.textLabel?.text = "Clear status"
      cell.textLabel?.textColor = .systemRed
      cell.accessibilityTraits.insert(.button)
      cell.selectionStyle = .default
    }
    return cell
  }

  override func tableView(_ tableView: UITableView, didSelectRowAt indexPath: IndexPath) {
    tableView.deselectRow(at: indexPath, animated: true)
    if indexPath.section == 2 {
      emojiField.text = presets[indexPath.row].0
      statusField.text = presets[indexPath.row].1
      changed()
    } else if indexPath.section == 3 {
      submit(clear: true)
    }
  }

  @objc private func changed() {
    navigationItem.rightBarButtonItem?.isEnabled =
      !(statusField.text ?? "").trimmingCharacters(in: .whitespacesAndNewlines).isEmpty
      || !(emojiField.text ?? "").isEmpty
  }

  @objc private func cancel() { onCancel() }
  @objc private func save() {
    guard navigationItem.rightBarButtonItem?.isEnabled == true else { return }
    submit(clear: false)
  }

  private func submit(clear: Bool) {
    onSet([
      "text": statusField.text ?? "", "emoji": emojiField.text ?? "",
      "duration": duration, "clear": clear,
      "expiresAt": duration == 0 ? datePicker.date.timeIntervalSince1970
        : Date().addingTimeInterval(TimeInterval(duration)).timeIntervalSince1970,
    ])
  }

  func textFieldShouldReturn(_ textField: UITextField) -> Bool {
    save()
    return true
  }
}

/// Open the system's complete emoji keyboard when it is enabled.
final class StatusEmojiTextField: UITextField {
  override var textInputContextIdentifier: String? { "buzz.status.emoji" }
  override var textInputMode: UITextInputMode? {
    UITextInputMode.activeInputModes.first { $0.primaryLanguage == "emoji" } ?? super.textInputMode
  }
}
