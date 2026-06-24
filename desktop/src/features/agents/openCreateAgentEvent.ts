const OPEN_CREATE_AGENT_EVENT = "buzz:open-create-agent";

let pendingOpenCreateAgent = false;

export function requestOpenCreateAgent() {
  pendingOpenCreateAgent = true;
  window.dispatchEvent(new Event(OPEN_CREATE_AGENT_EVENT));
}

export function consumePendingOpenCreateAgent() {
  if (!pendingOpenCreateAgent) {
    return false;
  }

  pendingOpenCreateAgent = false;
  return true;
}

export function subscribeOpenCreateAgent(handler: () => void) {
  function handleOpenCreateAgent() {
    pendingOpenCreateAgent = false;
    handler();
  }

  window.addEventListener(OPEN_CREATE_AGENT_EVENT, handleOpenCreateAgent);

  return () => {
    window.removeEventListener(OPEN_CREATE_AGENT_EVENT, handleOpenCreateAgent);
  };
}
