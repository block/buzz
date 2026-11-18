import SwiftUI
import WidgetKit

/// Lock Screen and Control Center controls that open Buzz to a quick action.
@main
struct BuzzControls: WidgetBundle {
  var body: some Widget {
    NewChatControl()
    MessageBestieControl()
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
