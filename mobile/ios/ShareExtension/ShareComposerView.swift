import SwiftUI
import UIKit

/// What the user is about to share, reduced to what the sheet can preview.
struct SharePreview {
  var text: String
  var fileNames: [String]
  var thumbnails: [UIImage]

  var hasFiles: Bool { !fileNames.isEmpty }
}

/// The share sheet's own composer: message text, a channel picker across the
/// user's communities, and Send. Mirrors the destinations the app offers.
struct ShareComposerView: View {
  let preview: SharePreview
  let targets: BuzzShareTargets.File
  let onSend: (BuzzShareTargets.Target, String) -> Void
  let onContinueInApp: (String) -> Void
  let onCancel: () -> Void

  @State private var message: String
  @State private var query = ""
  @State private var communityID: String
  @State private var selected: BuzzShareTargets.Target?

  private static let lastCommunityKey = "share.lastCommunityID"

  init(
    preview: SharePreview,
    targets: BuzzShareTargets.File,
    onSend: @escaping (BuzzShareTargets.Target, String) -> Void,
    onContinueInApp: @escaping (String) -> Void,
    onCancel: @escaping () -> Void
  ) {
    self.preview = preview
    self.targets = targets
    self.onSend = onSend
    self.onContinueInApp = onContinueInApp
    self.onCancel = onCancel
    _message = State(initialValue: preview.text)
    let remembered = UserDefaults.standard.string(forKey: Self.lastCommunityKey)
    let initial =
      targets.communities.first { $0.id == remembered }?.id
      ?? targets.communities.first?.id ?? ""
    _communityID = State(initialValue: initial)
  }

  private var filteredTargets: [BuzzShareTargets.Target] {
    let normalized = query.trimmingCharacters(in: .whitespacesAndNewlines).lowercased()
    return
      targets.targets
      .filter { $0.communityID == communityID }
      .filter { normalized.isEmpty || $0.label.lowercased().contains(normalized) }
      .sorted { ($0.lastMessageAt ?? 0) > ($1.lastMessageAt ?? 0) }
  }

  private var canSend: Bool {
    selected != nil
      && (preview.hasFiles
        || !message.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty)
  }

  var body: some View {
    NavigationStack {
      Group {
        if targets.communities.isEmpty {
          emptyState
        } else {
          composer
        }
      }
      .navigationTitle("Share to Buzz")
      .navigationBarTitleDisplayMode(.inline)
      .toolbar {
        ToolbarItem(placement: .cancellationAction) {
          Button("Cancel", action: onCancel)
        }
        if !targets.communities.isEmpty {
          ToolbarItem(placement: .confirmationAction) {
            // Opens Buzz with the message ready in the chosen channel; the
            // app does the sending, so the label must not promise it.
            Button("Continue") {
              if let selected {
                UserDefaults.standard.set(communityID, forKey: Self.lastCommunityKey)
                onSend(selected, message)
              }
            }
            .fontWeight(.semibold)
            .disabled(!canSend)
            .accessibilityHint("Opens Buzz with this message ready to send")
          }
        }
      }
    }
  }

  private var composer: some View {
    VStack(spacing: 0) {
      VStack(alignment: .leading, spacing: 8) {
        if !preview.thumbnails.isEmpty || preview.hasFiles {
          attachmentsRow
        }
        TextEditor(text: $message)
          .frame(minHeight: 56, maxHeight: 120)
          .padding(6)
          .background(Color(.secondarySystemBackground))
          .clipShape(RoundedRectangle(cornerRadius: 10))
          .overlay(alignment: .topLeading) {
            if message.isEmpty {
              Text("Add a message")
                .foregroundStyle(.tertiary)
                .padding(.horizontal, 11)
                .padding(.vertical, 14)
                .allowsHitTesting(false)
                .accessibilityHidden(true)
            }
          }
          .accessibilityLabel("Message")
      }
      .padding(.horizontal)
      .padding(.top, 8)

      if targets.communities.count > 1 {
        Picker("Community", selection: $communityID) {
          ForEach(targets.communities) { community in
            Text(community.name).tag(community.id)
          }
        }
        .pickerStyle(.menu)
        .padding(.horizontal)
        .padding(.top, 4)
        .onChange(of: communityID) { _ in selected = nil }
      }

      List {
        ForEach(filteredTargets) { target in
          Button {
            selected = target
          } label: {
            HStack {
              Image(systemName: target.isDM ? "bubble.left" : "number")
                .foregroundStyle(.secondary)
                .frame(width: 22)
                .accessibilityHidden(true)
              Text(target.label).lineLimit(1)
              Spacer()
              if selected?.id == target.id {
                Image(systemName: "checkmark")
                  .foregroundStyle(.tint)
                  .accessibilityHidden(true)
              }
            }
            .contentShape(Rectangle())
          }
          .buttonStyle(.plain)
          .accessibilityLabel(target.isDM ? "Direct message, \(target.label)" : target.label)
          .accessibilityAddTraits(selected?.id == target.id ? .isSelected : [])
        }
      }
      .listStyle(.plain)
      .searchable(text: $query, placement: .navigationBarDrawer(displayMode: .always), prompt: "Search channels")
      .overlay {
        if filteredTargets.isEmpty {
          if query.isEmpty {
            // No channels published for this community yet: never leave
            // Cancel as the only way out.
            VStack(spacing: 12) {
              Text("No channels yet").foregroundStyle(.secondary)
              Button("Continue in Buzz") { onContinueInApp(message) }
                .buttonStyle(.borderedProminent)
            }
          } else {
            Text("No matching channels").foregroundStyle(.secondary)
          }
        }
      }
    }
  }

  private var attachmentsRow: some View {
    ScrollView(.horizontal, showsIndicators: false) {
      HStack(spacing: 8) {
        ForEach(Array(preview.thumbnails.enumerated()), id: \.offset) { _, image in
          Image(uiImage: image)
            .resizable()
            .scaledToFill()
            .frame(width: 56, height: 56)
            .clipShape(RoundedRectangle(cornerRadius: 8))
        }
        ForEach(preview.fileNames.dropFirst(preview.thumbnails.count), id: \.self) { name in
          Label(name, systemImage: "doc")
            .lineLimit(1)
            .font(.footnote)
            .padding(.horizontal, 10)
            .frame(height: 56)
            .background(Color(.secondarySystemBackground))
            .clipShape(RoundedRectangle(cornerRadius: 8))
        }
      }
    }
    .accessibilityElement(children: .ignore)
    .accessibilityLabel(
      preview.fileNames.count == 1 ? "1 attachment" : "\(preview.fileNames.count) attachments"
    )
  }

  private var emptyState: some View {
    VStack(spacing: 12) {
      Image(systemName: "bubble.left.and.bubble.right")
        .font(.largeTitle)
        .foregroundStyle(.secondary)
        .accessibilityHidden(true)
      Text("Open Buzz once to load your channels")
        .multilineTextAlignment(.center)
      Button("Continue in Buzz") { onContinueInApp(message) }
        .buttonStyle(.borderedProminent)
    }
    .padding()
  }
}
