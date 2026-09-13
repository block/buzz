import Foundation

/// Verified local values used to specialize an ordinary notification as communication.
public struct BuzzCommunicationNotificationDescriptor: Equatable, Sendable {
  public let senderDisplayName: String
  public let senderIdentifier: String
  public let senderAvatarPNG: Data?
  public let messageBody: String
  public let conversationIdentifier: String
  public let conversationDisplayName: String?
  /// Verified recipients represented by the incoming message, excluding its sender.
  public let recipientCount: Int

  public init(
    senderDisplayName: String,
    senderIdentifier: String,
    senderAvatarPNG: Data?,
    messageBody: String,
    conversationIdentifier: String,
    conversationDisplayName: String?,
    recipientCount: Int
  ) {
    self.senderDisplayName = senderDisplayName
    self.senderIdentifier = senderIdentifier
    self.senderAvatarPNG = senderAvatarPNG
    self.messageBody = messageBody
    self.conversationIdentifier = conversationIdentifier
    self.conversationDisplayName = conversationDisplayName
    self.recipientCount = recipientCount
  }

  public init?(resolution: BuzzPushResolution) {
    guard let target = resolution.navigationTarget,
      let senderPubkey = resolution.senderPubkey,
      !senderPubkey.isEmpty,
      let conversationIdentifier = resolution.conversationIdentifier,
      !conversationIdentifier.isEmpty,
      let recipientCount = resolution.conversationRecipientCount,
      recipientCount > 0
    else { return nil }
    self.init(
      senderDisplayName: resolution.title,
      senderIdentifier: BuzzPushPresentationIdentity.sender(
        communityID: target.communityID,
        pubkey: senderPubkey
      ),
      senderAvatarPNG: resolution.senderAvatarPNG,
      messageBody: resolution.body,
      conversationIdentifier: conversationIdentifier,
      conversationDisplayName: resolution.conversationDisplayName,
      recipientCount: recipientCount
    )
  }
}

#if os(iOS)
  import Intents
  import UserNotifications

  /// Donates and applies Apple's supported Communication Notifications intent.
  public final class BuzzCommunicationNotificationPresenter {
    public typealias Donation = (INInteraction, @escaping (Error?) -> Void) -> Void
    public typealias ContentUpdate = (
      UNMutableNotificationContent,
      INSendMessageIntent
    ) throws -> UNNotificationContent

    private let donate: Donation
    private let updateContent: ContentUpdate

    public convenience init() {
      self.init(
        donate: { interaction, completion in
          interaction.donate(completion: completion)
        },
        updateContent: { content, intent in
          try content.updating(from: intent)
        }
      )
    }

    public init(
      donate: @escaping Donation,
      updateContent: @escaping ContentUpdate
    ) {
      self.donate = donate
      self.updateContent = updateContent
    }

    public func present(
      ordinaryContent: UNMutableNotificationContent,
      resolution: BuzzPushResolution,
      completion: @escaping (UNNotificationContent) -> Void
    ) {
      guard let descriptor = BuzzCommunicationNotificationDescriptor(resolution: resolution) else {
        completion(ordinaryContent)
        return
      }
      let intent = Self.makeIntent(descriptor)
      let interaction = INInteraction(intent: intent, response: nil)
      interaction.direction = .incoming
      donate(interaction) { [updateContent] error in
        guard error == nil,
          let specialized = try? updateContent(ordinaryContent, intent)
        else {
          completion(ordinaryContent)
          return
        }
        completion(specialized)
      }
    }

    public static func makeIntent(
      _ descriptor: BuzzCommunicationNotificationDescriptor
    ) -> INSendMessageIntent {
      let senderAvatar = descriptor.senderAvatarPNG.map(INImage.init(imageData:))
      let sender = INPerson(
        personHandle: INPersonHandle(value: descriptor.senderIdentifier, type: .unknown),
        nameComponents: nil,
        displayName: descriptor.senderDisplayName,
        image: senderAvatar,
        contactIdentifier: nil,
        customIdentifier: descriptor.senderIdentifier,
        isMe: false,
        suggestionType: .none
      )
      let intent = INSendMessageIntent(
        recipients: nil,
        outgoingMessageType: .outgoingMessageText,
        content: descriptor.messageBody,
        speakableGroupName: descriptor.conversationDisplayName.map {
          INSpeakableString(spokenPhrase: $0)
        },
        conversationIdentifier: descriptor.conversationIdentifier,
        serviceName: "Buzz",
        sender: sender,
        attachments: nil
      )
      if descriptor.conversationDisplayName != nil {
        let donationMetadata = INSendMessageIntentDonationMetadata()
        donationMetadata.recipientCount = descriptor.recipientCount
        intent.donationMetadata = donationMetadata
        if let senderAvatar {
          // Communication Notifications render a group conversation's image
          // from the speakable-group parameter rather than INPerson.image.
          // Buzz channels do not have a separate avatar, so use the verified
          // sender thumbnail for the visible incoming-message avatar.
          intent.setImage(senderAvatar, forParameterNamed: \.speakableGroupName)
        }
      }
      return intent
    }
  }
#endif
