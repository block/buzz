import SwiftUI
import WidgetKit

/// Lock Screen widgets, plus Lock Screen and Control Center controls, that
/// open Buzz to a quick action.
@main
struct BuzzControls: WidgetBundle {
  var body: some Widget {
    NewChatWidget()
    MessageBestieWidget()
    if #available(iOS 18.0, *) {
      NewChatControl()
      MessageBestieControl()
    }
  }
}

// MARK: - Lock Screen widgets

struct NewChatWidget: Widget {
  var body: some WidgetConfiguration {
    ShortcutWidget.configuration(
      kind: "xyz.block.buzz.widget.new-chat",
      type: "new-chat",
      title: "New chat",
      description: "Start a Solo chat.",
      systemImage: "square.and.pencil"
    )
  }
}

struct MessageBestieWidget: Widget {
  var body: some WidgetConfiguration {
    ShortcutWidget.configuration(
      kind: "xyz.block.buzz.widget.message-bestie",
      type: "message-bestie",
      title: "Message Bestie",
      description: "Open your DM with your Bestie.",
      systemImage: "bubble.left.and.text.bubble.right"
    )
  }
}

/// A circular Lock Screen widget that opens a quick action when tapped.
/// Its content never changes, so its timeline is a single entry.
private enum ShortcutWidget {
  struct Entry: TimelineEntry {
    let date: Date
  }

  struct Provider: TimelineProvider {
    func placeholder(in context: Context) -> Entry { Entry(date: .now) }

    func getSnapshot(in context: Context, completion: @escaping (Entry) -> Void) {
      completion(Entry(date: .now))
    }

    func getTimeline(in context: Context, completion: @escaping (Timeline<Entry>) -> Void) {
      completion(Timeline(entries: [Entry(date: .now)], policy: .never))
    }
  }

  struct Face: View {
    let title: String
    let systemImage: String

    var body: some View {
      ZStack {
        AccessoryWidgetBackground()
        Image(systemName: systemImage)
          .font(.title2)
          .accessibilityHidden(true)
      }
      .accessibilityElement(children: .ignore)
      .accessibilityLabel(title)
      .accessibilityAddTraits(.isButton)
      .widgetBackground()
    }
  }

  static func configuration(
    kind: String,
    type: String,
    title: LocalizedStringResource,
    description: LocalizedStringResource,
    systemImage: String
  ) -> some WidgetConfiguration {
    StaticConfiguration(kind: kind, provider: Provider()) { _ in
      Face(title: String(localized: title), systemImage: systemImage)
        .widgetURL(ShortcutLink.url(type))
    }
    .configurationDisplayName(title)
    .description(description)
    .supportedFamilies([.accessoryCircular])
  }
}

private extension View {
  /// iOS 17 requires a container background; earlier versions draw none.
  @ViewBuilder
  func widgetBackground() -> some View {
    if #available(iOS 17.0, *) {
      containerBackground(.clear, for: .widget)
    } else {
      self
    }
  }
}

// MARK: - Controls

@available(iOS 18.0, *)
struct NewChatControl: ControlWidget {
  var body: some ControlWidgetConfiguration {
    StaticControlConfiguration(kind: "xyz.block.buzz.control.new-chat") {
      ControlWidgetButton(action: NewChatControlIntent()) {
        Label("New chat", systemImage: "square.and.pencil")
      }
    }
    .displayName("New chat")
    .description("Start a Solo chat.")
  }
}

@available(iOS 18.0, *)
struct MessageBestieControl: ControlWidget {
  var body: some ControlWidgetConfiguration {
    StaticControlConfiguration(kind: "xyz.block.buzz.control.message-bestie") {
      ControlWidgetButton(action: MessageBestieControlIntent()) {
        Label("Message Bestie", systemImage: "bubble.left.and.text.bubble.right")
      }
    }
    .displayName("Message Bestie")
    .description("Open your DM with your Bestie.")
  }
}
