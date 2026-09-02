(() => {
  window.buzzCanvasWidgets = window.buzzCanvasWidgets || {};
  window.buzzCanvasWidgets.support = {
    companions: { bugReporter: renderBugCompanion },
    renderers: {
      bugReporter: renderBugReporter,
      knownIssues: renderKnownIssues,
      releaseNotes: renderReleaseNotes,
    },
  };

  function renderReleaseNotes(data, { element, icon }) {
    const section = element("section", "release-notes", {
      ariaLabel: "Latest Acorn release notes",
      testId: "project-canvas-release-notes",
    });
    const header = element("header", "release-header");
    const copy = element("div", "");
    copy.append(
      element("h3", "", { text: data.product }),
      element("p", "muted", { text: "Released today · Product update" }),
    );
    header.append(
      icon("↗", "release-icon"),
      copy,
      element("span", "live-badge", { text: "Live" }),
    );
    section.append(header);
    data.items.forEach((item, index) => {
      const row = element("article", "release-row");
      const rowCopy = element("div", "");
      rowCopy.append(
        element("h4", "", { text: item.title }),
        element("p", "muted", { text: item.detail }),
      );
      row.append(
        icon(["⚡", "◉", "✦"][index], `release-tone-${index}`),
        rowCopy,
      );
      section.append(row);
    });
    return section;
  }

  function renderKnownIssues(data, { element }) {
    const section = element("section", "known-issues", {
      ariaLabel: "Known product issues",
      testId: "project-canvas-known-issues",
    });
    const header = element("header", "issues-header");
    const title = element("div", "");
    title.append(
      element("h3", "", { text: "Known issues" }),
      element("p", "muted", { text: "Support noticeboard" }),
    );
    header.append(title, element("span", "muted", { text: "Updated 12m ago" }));
    section.append(header);
    const grid = element("div", "issue-grid");
    data.issues.forEach((issue, index) => {
      const note = element(
        "article",
        `issue-note tone-${issue.tone}${index === 2 ? " wide" : ""}`,
      );
      const noteTitle = element("div", "issue-title");
      noteTitle.append(
        element("h4", "", { text: issue.title }),
        element("span", "", { text: issue.id }),
      );
      note.append(
        noteTitle,
        element("p", "", { text: issue.detail }),
        element("strong", "issue-status", { text: issue.status }),
      );
      grid.append(note);
    });
    section.append(grid);
    return section;
  }

  function renderBugReporter(data, { element, icon }) {
    const form = element("form", "bug-reporter", {
      testId: "project-canvas-support-bug-reporter",
    });
    const header = element("header", "bug-header");
    const copy = element("div", "");
    copy.append(
      element("h3", "", { text: "Report a problem" }),
      element("p", "muted", {
        text: `Acorn support · ${data.responseTime}`,
      }),
    );
    header.append(icon("✦", "bug-icon"), copy);
    const editor = element("div", "bug-editor");
    const textarea = element("textarea", "", {
      ariaLabel: "Describe a support issue",
      maxlength: "1900",
      placeholder: "What happened? Include what you expected to see...",
      testId: "project-canvas-support-bug-input",
    });
    const submit = element("button", "submit-button", {
      ariaLabel: "Submit support report",
      testId: "project-canvas-support-bug-submit",
      text: "Send",
      type: "submit",
    });
    submit.disabled = true;
    textarea.addEventListener("input", () => {
      submit.disabled = !textarea.value.trim();
    });
    editor.append(textarea, submit);
    form.append(header, editor);
    function showOutcome(title, detail) {
      const success = element("div", "bug-success", {
        testId: "project-canvas-support-bug-success",
      });
      success.append(
        icon("✓", "success-circle"),
        element("h4", "", { text: title }),
        element("p", "muted", { text: detail }),
      );
      form.replaceChildren(header, success);
    }
    form.addEventListener("submit", (event) => {
      event.preventDefault();
      const report = textarea.value.trim();
      if (!report) return;
      const sdk = window.buzzCanvas?.sdk;
      const canDm =
        !!sdk?.data &&
        sdk.capabilities().includes("app.dm.send") &&
        sdk.capabilities().includes("project.metadata.read");
      if (!canDm) {
        showOutcome(
          "Report staged",
          "We'll check for matching issues before filing.",
        );
        return;
      }
      submit.disabled = true;
      sdk.data
        .query("project.metadata")
        .then((result) => {
          const owner = result?.data ? String(result.data.owner || "") : "";
          if (!/^[0-9a-f]{64}$/i.test(owner)) {
            throw new Error("Project owner is unavailable.");
          }
          return sdk.data.command("dm.send", {
            message: `Support report: ${report}`,
            pubkey: owner,
          });
        })
        .then(() => {
          showOutcome(
            "Report sent",
            "Delivered to the project owner as a direct message.",
          );
        })
        .catch(() => {
          // Failures surface through the host's command toast; re-enable so
          // the report stays retryable.
          submit.disabled = false;
        });
    });
    return form;
  }

  function renderBugCompanion(widget, { element, resolveAsset }) {
    const wrapper = element("div", "companion bug-companion", {
      testId: "project-canvas-bug-gloopie-companion",
    });
    const video = element("video", "gloopie-video", {
      ariaLabel: "Bug report helper",
      autoplay: "",
      loop: "",
      muted: "",
      playsinline: "",
      poster: resolveAsset(widget.data.gloopiePoster),
      testId: "project-canvas-gloopie",
    });
    video.dataset.berdAvatarId = "gloopies-22";
    video.muted = true;
    video.src = resolveAsset(widget.data.gloopie);
    wrapper.append(video);
    return wrapper;
  }
})();
