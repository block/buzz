const OPEN_CREATE_AGENT_EVENT = "buzz:open-create-agent";

export type OpenCreateAgentOptions = {
  channelId?: string;
  channelName?: string;
};

let pendingOpenCreateAgent: OpenCreateAgentOptions | null = null;

export function requestOpenCreateAgent(options: OpenCreateAgentOptions = {}) {
  pendingOpenCreateAgent = options;
  window.dispatchEvent(
    new CustomEvent<OpenCreateAgentOptions>(OPEN_CREATE_AGENT_EVENT, {
      detail: options,
    }),
  );
}

export function consumePendingOpenCreateAgent() {
  const pending = pendingOpenCreateAgent;
  pendingOpenCreateAgent = null;
  return pending;
}

export function subscribeOpenCreateAgent(
  handler: (options: OpenCreateAgentOptions) => void,
) {
  function handleOpenCreateAgent(event: Event) {
    pendingOpenCreateAgent = null;
    handler(
      event instanceof CustomEvent
        ? (event.detail as OpenCreateAgentOptions)
        : {},
    );
  }

  window.addEventListener(OPEN_CREATE_AGENT_EVENT, handleOpenCreateAgent);

  return () => {
    window.removeEventListener(OPEN_CREATE_AGENT_EVENT, handleOpenCreateAgent);
  };
}
