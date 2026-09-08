import {
  IconHash,
  IconMessageCircle,
  IconMessagePlus,
  IconMessages,
  IconPlus,
} from "@tabler/icons-react";
import { IconButton } from "@/shared/ui/IconButton";
import { NavigationItem } from "@/shared/ui/NavigationItem";
import { NavigationSection } from "@/shared/ui/NavigationSection";
import { SearchField } from "@/shared/ui/SearchField";

type NavigatorConversation = {
  id: string;
  name: string;
  channelType: string;
};

type NavigatorSession = {
  channelId: string;
  originChannelId?: string;
};

export function MessagesNavigator({
  conversations,
  sessions,
  selectedId,
  onOpenRoom,
  onOpenDirectMessage,
  onOpenSession,
  onStartSession,
  onStartStandaloneSession,
  query,
  onQueryChange,
}: {
  conversations: NavigatorConversation[];
  sessions: NavigatorSession[];
  selectedId?: string;
  onOpenRoom: (channelId: string) => void;
  onOpenDirectMessage: (channelId: string) => void;
  onOpenSession: (channelId: string) => void;
  onStartSession: (channelId: string) => void;
  onStartStandaloneSession: () => void;
  query: string;
  onQueryChange: (query: string) => void;
}) {
  const sessionsById = new Map(
    sessions.map((session) => [session.channelId, session]),
  );
  const conversationsById = new Map(
    conversations.map((conversation) => [conversation.id, conversation]),
  );
  const normalizedQuery = query.trim().toLowerCase();
  const matches = (value: string) =>
    !normalizedQuery || value.toLowerCase().includes(normalizedQuery);
  const rooms = conversations.filter(
    (conversation) =>
      conversation.channelType !== "dm" && !sessionsById.has(conversation.id),
  );
  const directMessages = conversations.filter(
    (conversation) => conversation.channelType === "dm",
  );
  const sessionConversations = sessions
    .map((session) => ({
      session,
      conversation: conversationsById.get(session.channelId),
    }))
    .filter(
      (
        entry,
      ): entry is {
        session: NavigatorSession;
        conversation: NavigatorConversation;
      } => Boolean(entry.conversation),
    );

  return (
    <aside className="workspace-navigator" aria-label="Messages">
      <header className="panel-heading navigator-heading">
        <SearchField
          label="Find a Room, person, or Session"
          placeholder="Search"
          value={query}
          onValueChange={onQueryChange}
        />
      </header>
      <div className="navigator-list">
        <NavigationSection label="Rooms">
          {rooms
            .filter((room) => {
              const roomSessions = sessions.filter(
                (session) => session.originChannelId === room.id,
              );
              return (
                matches(room.name) ||
                roomSessions.some((session) =>
                  matches(conversationsById.get(session.channelId)?.name ?? ""),
                )
              );
            })
            .map((room) => {
              const roomSessions = sessions
                .map((session) => ({
                  session,
                  conversation: conversationsById.get(session.channelId),
                }))
                .filter(
                  (
                    entry,
                  ): entry is {
                    session: NavigatorSession;
                    conversation: NavigatorConversation;
                  } =>
                    entry.session.originChannelId === room.id &&
                    Boolean(entry.conversation),
                )
                .filter(({ conversation }) => matches(conversation.name));
              return (
                <div className="channel-cluster" key={room.id}>
                  <div className="channel-main-row">
                    <NavigationItem
                      label={room.name}
                      icon={
                        <IconHash size={15} stroke={1.6} aria-hidden="true" />
                      }
                      selected={selectedId === room.id}
                      onClick={() => onOpenRoom(room.id)}
                    />
                    <IconButton
                      aria-label={`Start Session from ${room.name}`}
                      icon={
                        <IconPlus size={14} stroke={1.7} aria-hidden="true" />
                      }
                      size="compact"
                      onClick={() => onStartSession(room.id)}
                    />
                  </div>
                  {roomSessions.length ? (
                    <div className="session-children">
                      {roomSessions.map(({ conversation }) => (
                        <div className="session-child" key={conversation.id}>
                          <NavigationItem
                            label={conversation.name}
                            inset
                            selected={selectedId === conversation.id}
                            onClick={() => onOpenSession(conversation.id)}
                          />
                        </div>
                      ))}
                    </div>
                  ) : null}
                </div>
              );
            })}
          {rooms.filter((room) => matches(room.name)).length === 0 ? (
            <p className="navigation-empty text-body-sm text-tertiary">
              {normalizedQuery ? "No Rooms match." : "No Rooms yet."}
            </p>
          ) : null}
        </NavigationSection>

        <NavigationSection label="Direct messages">
          {directMessages
            .filter((message) => matches(message.name))
            .map((message) => (
              <NavigationItem
                key={message.id}
                label={message.name}
                icon={
                  <IconMessageCircle
                    size={15}
                    stroke={1.6}
                    aria-hidden="true"
                  />
                }
                selected={selectedId === message.id}
                onClick={() => onOpenDirectMessage(message.id)}
              />
            ))}
          {directMessages.filter((message) => matches(message.name)).length ===
          0 ? (
            <p className="navigation-empty text-body-sm text-tertiary">
              {normalizedQuery ? "No people match." : "No direct messages yet."}
            </p>
          ) : null}
        </NavigationSection>

        <NavigationSection
          label="Sessions"
          action={
            <IconButton
              aria-label="Start a standalone Session"
              icon={
                <IconMessagePlus size={14} stroke={1.7} aria-hidden="true" />
              }
              size="compact"
              onClick={onStartStandaloneSession}
            />
          }
        >
          {sessionConversations
            .filter(
              ({ conversation, session }) =>
                !session.originChannelId && matches(conversation.name),
            )
            .map(({ conversation }) => (
              <NavigationItem
                key={conversation.id}
                label={conversation.name}
                icon={
                  <IconMessages size={15} stroke={1.6} aria-hidden="true" />
                }
                selected={selectedId === conversation.id}
                onClick={() => onOpenSession(conversation.id)}
              />
            ))}
          {sessionConversations.filter(
            ({ conversation, session }) =>
              !session.originChannelId && matches(conversation.name),
          ).length === 0 ? (
            <p className="navigation-empty text-body-sm text-tertiary">
              {normalizedQuery
                ? "No Sessions match."
                : "Start a Session for focused work."}
            </p>
          ) : null}
        </NavigationSection>
      </div>
    </aside>
  );
}
