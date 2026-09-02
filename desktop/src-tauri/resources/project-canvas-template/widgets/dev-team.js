(() => {
  window.buzzCanvasWidgets = window.buzzCanvasWidgets || {};
  window.buzzCanvasWidgets.devTeam = {
    renderers: {
      activeChannels: renderActiveChannels,
      clientTime: renderClientTime,
      meetings: renderMeetings,
      reviews: renderReviews,
      tasks: renderTasks,
    },
  };

  // One live subscription per widget type: re-rendering a widget replaces its
  // subscription instead of leaking the previous one.
  const liveStops = new Map();

  function sdkApi() {
    const sdk = window.buzzCanvas?.sdk;
    return sdk?.data && typeof sdk.data.liveQuery === "function" ? sdk : null;
  }

  function startLiveList(options, element) {
    const previousStop = liveStops.get(options.key);
    if (previousStop) previousStop();
    const sdk = sdkApi();
    if (!sdk?.capabilities().includes(options.capability)) {
      liveStops.delete(options.key);
      options.container.replaceChildren(
        renderSnapshotState("unavailable", options.noun, element),
      );
      return;
    }
    options.container.replaceChildren(
      renderSnapshotState("loading", options.noun, element),
    );
    const stop = sdk.data.liveQuery(options.query, options.params, (result) => {
      if (!result || result.status === "loading") {
        options.container.replaceChildren(
          renderSnapshotState("loading", options.noun, element),
        );
        return;
      }
      if (result.status === "error") {
        options.container.replaceChildren(
          renderSnapshotState("error", options.noun, element),
        );
        return;
      }
      const rows = Array.isArray(result.data) ? result.data : [];
      if (rows.length === 0) {
        options.container.replaceChildren(
          renderSnapshotState("empty", options.noun, element),
        );
        return;
      }
      options.container.replaceChildren();
      options.render(rows, options.container, sdk);
    });
    liveStops.set(options.key, stop);
  }

  function renderActiveChannels(_data, api) {
    const { element } = api;
    const list = element("div", "active-channels", {
      testId: "project-canvas-active-channels",
    });
    startLiveList(
      {
        capability: "project.channels.read",
        container: list,
        key: "activeChannels",
        noun: "channels",
        params: {},
        query: "project.channels.list",
        render: (channels, container, sdk) => {
          for (const channel of channels) {
            container.append(sdk.ui.channelRow({ channel }));
          }
        },
      },
      element,
    );
    return list;
  }

  function renderReviews(_data, api) {
    const { element } = api;
    const section = element("section", "reviews", {
      ariaLabel: "Reviews waiting on the team",
      testId: "project-canvas-reviews",
    });
    startLiveList(
      {
        capability: "project.reviews.read",
        container: section,
        key: "reviews",
        noun: "reviews",
        params: { status: "Open" },
        query: "project.reviews.list",
        render: (reviews, container, sdk) => {
          const intro = element("header", "reviews-intro");
          intro.append(
            element("div", "", { text: "Waiting on review" }),
            element("strong", "count-badge", {
              text: `${reviews.length} open`,
            }),
          );
          container.append(intro);
          for (const review of reviews) {
            container.append(sdk.ui.reviewRow({ review }));
          }
        },
      },
      element,
    );
    return section;
  }

  function renderTasks(_data, api) {
    const { element } = api;
    const section = element("section", "tasks", {
      ariaLabel: "Project tasks",
      testId: "project-canvas-tasks",
    });
    startLiveList(
      {
        capability: "project.tasks.read",
        container: section,
        key: "tasks",
        noun: "tasks",
        params: { limit: 8 },
        query: "project.tasks.list",
        render: (tasks, container, sdk) => {
          for (const task of tasks) {
            container.append(renderTaskRow(task, sdk, element));
          }
        },
      },
      element,
    );
    return section;
  }

  function renderTaskRow(task, sdk, element) {
    const title = String(task.title || "Untitled task");
    const status = String(task.status || "Triage");
    const row = element("article", "task-row", {
      testId: `project-canvas-task-${task.displayId || task.id.slice(0, 8)}`,
    });
    const summary = element("div", "task-summary");
    summary.append(
      element("span", "task-id", { text: task.displayId }),
      element("strong", "task-title", { text: title }),
    );
    const pill = element("span", "status-pill", { text: status });
    pill.dataset.status = status.toLowerCase().replaceAll(" ", "-");
    row.append(summary, pill);

    const actions = element("div", "task-actions");
    if (sdk.capabilities().includes("app.open")) {
      actions.append(
        taskActionButton(element, `Open ${title}`, "Open", () =>
          sdk.app.open({ id: task.id, type: "task" }),
        ),
      );
    }
    if (sdk.capabilities().includes("project.tasks.write")) {
      const finished = status === "Done" || status === "Closed";
      actions.append(
        taskActionButton(
          element,
          finished ? `Reopen ${title}` : `Mark ${title} done`,
          finished ? "Reopen" : "Mark done",
          () =>
            sdk.data.command("tasks.setStatus", {
              id: task.id,
              status: finished ? "open" : "done",
            }),
        ),
      );
      if (!finished && (task.assignees || []).length === 0) {
        actions.append(
          taskActionButton(
            element,
            `Assign ${title} to me`,
            "Assign to me",
            () => sdk.data.command("tasks.assign", { id: task.id }),
          ),
        );
      }
    }
    if (actions.childElementCount > 0) row.append(actions);
    return row;
  }

  function taskActionButton(element, ariaLabel, label, run) {
    const button = element("button", "small-button", {
      ariaLabel,
      text: label,
      type: "button",
    });
    button.addEventListener("click", () => {
      button.disabled = true;
      // Failures surface through the host's command toast; re-enable so the
      // action stays retryable.
      run()
        .catch(() => {})
        .finally(() => {
          button.disabled = false;
        });
    });
    return button;
  }

  function renderSnapshotState(status, noun, element) {
    const messages = {
      empty: `No ${noun} to show`,
      error: `Could not load ${noun}`,
      loading: `Loading ${noun}…`,
      unavailable: `${noun[0].toUpperCase()}${noun.slice(1)} access unavailable`,
    };
    const container = element("div", "snapshot-state", {
      ariaLabel: messages[status],
      text: messages[status],
    });
    container.dataset.snapshotState = status;
    return container;
  }

  function renderClientTime(data, { element }) {
    const section = element("section", "client-time", {
      ariaLabel: "Client time tracking",
      testId: "project-canvas-contractor-time-tracking",
    });
    const summary = element("div", "time-summary");
    summary.append(
      element("p", "eyebrow", { text: "Weekly capacity" }),
      element("strong", "time-total", { text: data.booked }),
      element("span", "muted", { text: ` of ${data.capacity}` }),
    );
    const capacity = element("div", "capacity-bar");
    for (const client of data.clients) {
      const segment = element("span", "capacity-segment");
      segment.style.width = `${client.share}%`;
      segment.style.backgroundColor = client.color;
      capacity.append(segment);
    }
    summary.append(
      capacity,
      element("p", "capacity-note", { text: "77% booked · 9h 15m open" }),
    );
    section.append(summary);
    data.clients.forEach((client) => {
      const row = element("div", "client-row");
      const mark = element("span", "client-mark");
      mark.style.backgroundColor = client.color;
      const copy = element("div", "client-copy");
      copy.append(
        element("strong", "", { text: client.name }),
        element("span", "muted", { text: client.project }),
      );
      row.append(
        mark,
        copy,
        element("strong", "client-hours", { text: client.time }),
      );
      section.append(row);
    });
    return section;
  }

  function renderMeetings(data, api) {
    const { element, icon } = api;
    const section = element("section", "meetings", {
      ariaLabel: "Team meetings",
      testId: "project-canvas-meetings",
    });
    section.append(element("p", "eyebrow", { text: "Previous" }));
    const previous = element("div", "meeting-previous", {
      testId: "project-canvas-meeting-previous",
    });
    const copy = element("div", "meeting-copy");
    copy.append(
      element("strong", "", { text: data.previous.title }),
      element("span", "muted", {
        text: `${data.previous.time} · ${data.previous.duration}`,
      }),
    );
    const actions = element("div", "meeting-actions");
    const notes = element("button", "small-button", {
      text: "Notes",
      type: "button",
    });
    const recording = element("button", "small-button", {
      text: "Recording",
      type: "button",
    });
    notes.addEventListener("click", () => showMeetingNotes(data.previous, api));
    recording.addEventListener("click", () =>
      showMeetingRecording(data.previous, api),
    );
    actions.append(notes, recording);
    previous.append(icon("✓", "success"), copy, actions);
    section.append(
      previous,
      element("p", "eyebrow upcoming-label", { text: "Coming up" }),
    );
    const upcoming = element("ol", "upcoming-meetings", {
      ariaLabel: "Upcoming meetings",
    });
    data.upcoming.forEach((meeting) => {
      const row = element("li", "upcoming-row", {
        testId: "project-canvas-meeting-upcoming",
      });
      const time = element("div", "meeting-time");
      time.append(
        element("strong", "", { text: meeting.day }),
        element("span", "", { text: meeting.time }),
      );
      const details = element("div", "meeting-copy");
      details.append(
        element("strong", "", { text: meeting.title }),
        element("span", "muted", { text: meeting.duration }),
      );
      row.append(
        time,
        details,
        element("span", "scheduled", { text: "Scheduled" }),
      );
      upcoming.append(row);
    });
    section.append(upcoming);
    return section;
  }

  function showMeetingNotes(meeting, { element, showDialog }) {
    const body = element("div", "meeting-detail", {
      testId: "meeting-notes-detail",
    });
    body.append(
      element("p", "", { text: `${meeting.time} · ${meeting.duration}` }),
    );
    const list = element("ul", "notes-list");
    [
      "Ship the Canvas tab in the next desktop release.",
      "Keep project widgets local-only for the demo.",
      "Recheck mobile spacing before the final walkthrough.",
    ].forEach((note) => {
      list.append(element("li", "", { text: note }));
    });
    body.append(list);
    showDialog(`${meeting.title} notes`, body, "meeting-detail-dialog");
  }

  function showMeetingRecording(meeting, { element, showDialog }) {
    const body = element("div", "meeting-detail", {
      testId: "meeting-recording-detail",
    });
    body.append(
      element("p", "", { text: `${meeting.time} · ${meeting.duration}` }),
    );
    const screen = element("div", "recording-screen");
    const play = element("button", "play-button", {
      ariaLabel: "Play recording",
      text: "▶",
      type: "button",
    });
    play.addEventListener("click", () => {
      const playing = play.getAttribute("aria-label") === "Pause recording";
      play.setAttribute(
        "aria-label",
        playing ? "Play recording" : "Pause recording",
      );
      play.textContent = playing ? "▶" : "Ⅱ";
    });
    screen.append(play);
    body.append(
      screen,
      element("p", "recording-time", { text: "00:00 ━━━━━━━━━ 42:18" }),
    );
    showDialog(`${meeting.title} recording`, body, "meeting-detail-dialog");
  }
})();
