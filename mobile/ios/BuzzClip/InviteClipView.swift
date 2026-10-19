import StoreKit
import SwiftUI

/// The clip's only screen: who is inviting you, and a Join button.
struct InviteClipView: View {
  @ObservedObject var model: InviteClipModel

  var body: some View {
    VStack(spacing: 24) {
      Spacer(minLength: 0)
      switch model.phase {
      case .waitingForInvocation:
        placeholder(
          symbol: "qrcode.viewfinder",
          title: "Scan a Buzz invite",
          detail: "Open this App Clip from an invite QR code or link to join a community."
        )
      case .unsupported:
        placeholder(
          symbol: "link.badge.plus",
          title: "Not a Buzz invite",
          detail: "This link is not a community invite. Ask for a fresh invite link."
        )
      case .ready(let invite), .saved(let invite), .failed(let invite, _):
        communityCard(invite)
      }
      Spacer(minLength: 0)
      footer
    }
    .padding(24)
    .appStoreOverlay(isPresented: $model.showAppStoreOverlay) {
      SKOverlay.AppClipConfiguration(position: .bottom)
    }
  }

  private func communityCard(_ invite: BuzzInviteLink) -> some View {
    VStack(spacing: 16) {
      icon
      Text(model.relayInfo?.name ?? invite.communityName)
        .font(.title.bold())
        .multilineTextAlignment(.center)
      Text(invite.host)
        .font(.subheadline.monospaced())
        .foregroundStyle(.secondary)
      if let description = model.relayInfo?.description {
        Text(description)
          .font(.body)
          .multilineTextAlignment(.center)
          .foregroundStyle(.secondary)
      } else if model.relayInfoFailed {
        Text("Community details unavailable — showing the invite link's relay.")
          .font(.footnote)
          .multilineTextAlignment(.center)
          .foregroundStyle(.tertiary)
      } else {
        ProgressView().accessibilityLabel("Loading community details")
      }
    }
    .accessibilityElement(children: .combine)
  }

  @ViewBuilder
  private var icon: some View {
    let size: CGFloat = 88
    if let url = model.relayInfo?.icon {
      AsyncImage(url: url) { image in
        image.resizable().scaledToFill()
      } placeholder: {
        iconFallback
      }
      .frame(width: size, height: size)
      .clipShape(RoundedRectangle(cornerRadius: 20, style: .continuous))
      .accessibilityHidden(true)
    } else {
      iconFallback.frame(width: size, height: size).accessibilityHidden(true)
    }
  }

  private var iconFallback: some View {
    RoundedRectangle(cornerRadius: 20, style: .continuous)
      .fill(Color.accentColor.opacity(0.15))
      .overlay(Image(systemName: "person.3.fill").font(.largeTitle).foregroundStyle(Color.accentColor))
  }

  @ViewBuilder
  private var footer: some View {
    switch model.phase {
    case .ready:
      Button(action: model.join) {
        Text("Join").frame(maxWidth: .infinity)
      }
      .buttonStyle(.borderedProminent)
      .controlSize(.large)
    case .saved:
      VStack(spacing: 8) {
        Label("Invite saved", systemImage: "checkmark.circle.fill")
          .font(.headline)
          .foregroundStyle(.green)
        Text("Install Buzz to finish joining — the app picks this invite up on its first launch.")
          .font(.footnote)
          .multilineTextAlignment(.center)
          .foregroundStyle(.secondary)
        Button("Show install option") { model.showAppStoreOverlay = true }
          .buttonStyle(.bordered)
      }
    case .failed(_, let message):
      VStack(spacing: 8) {
        Text(message)
          .font(.footnote)
          .multilineTextAlignment(.center)
          .foregroundStyle(.red)
        Button("Try again", action: model.join)
          .buttonStyle(.borderedProminent)
          .controlSize(.large)
      }
    case .waitingForInvocation, .unsupported:
      EmptyView()
    }
  }

  private func placeholder(symbol: String, title: String, detail: String) -> some View {
    VStack(spacing: 12) {
      Image(systemName: symbol).font(.system(size: 56)).foregroundStyle(.secondary).accessibilityHidden(true)
      Text(title).font(.title2.bold())
      Text(detail).font(.body).multilineTextAlignment(.center).foregroundStyle(.secondary)
    }
    .accessibilityElement(children: .combine)
  }
}
