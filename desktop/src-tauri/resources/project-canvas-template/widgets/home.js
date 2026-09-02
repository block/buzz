(() => {
  window.buzzCanvasWidgets = window.buzzCanvasWidgets || {};
  window.buzzCanvasWidgets.home = {
    companions: {
      choreBoard: renderHenryCompanion,
      homeSchedule: renderHomeScheduleCompanion,
    },
    renderers: {
      choreBoard: {
        render: renderChoreBoard,
        update: updateChoreBoard,
      },
      familyLocations: renderFamilyLocations,
      frontYardCamera: renderFrontYardCamera,
      homeSchedule: renderHomeSchedule,
    },
  };

  function renderChoreBoard(data, { element }) {
    const board = element("div", "chore-board", {
      testId: "project-canvas-chore-board",
    });
    for (const group of data.groups) {
      const section = element("section", "chore-group");
      const heading = element("h3", "member-heading");
      const avatar = element("span", "avatar initials", {
        testId: `project-canvas-chore-member-${group.member.toLowerCase()}-avatar`,
        text: group.member.slice(0, 1),
      });
      avatar.style.backgroundColor = group.color;
      heading.append(avatar, document.createTextNode(group.member));
      section.append(heading);
      for (const chore of group.chores) {
        const id = `${group.member}-${chore}`
          .toLowerCase()
          .replace(/[^a-z0-9]+/g, "-");
        const label = element("label", "chore-row");
        const input = element("input", "", {
          "aria-label": `${chore} for ${group.member}`,
          testId: `project-canvas-chore-${id}`,
          type: "checkbox",
        });
        input.checked = group.completed.includes(chore);
        const text = element("span", "", { text: chore });
        input.addEventListener("change", () => {
          text.classList.toggle("completed", input.checked);
          group.completed = input.checked
            ? [...new Set([...group.completed, chore])]
            : group.completed.filter((candidate) => candidate !== chore);
        });
        text.classList.toggle("completed", input.checked);
        label.append(input, text);
        section.append(label);
      }
      board.append(section);
    }
    return board;
  }

  function updateChoreBoard(board, data, previousData, api) {
    const replacement = renderChoreBoard(data, api);
    board.replaceChildren(...replacement.childNodes);
    board.dataset.previousCompleted = String(completedCount(previousData));
    board.dataset.completed = String(completedCount(data));
    board.classList.remove("widget-data-updated");
    void board.offsetWidth;
    board.classList.add("widget-data-updated");
    board.addEventListener(
      "animationend",
      () => board.classList.remove("widget-data-updated"),
      { once: true },
    );
    return board;
  }

  function completedCount(data) {
    return (data?.groups || []).reduce(
      (total, group) => total + (group.completed || []).length,
      0,
    );
  }

  function renderHomeSchedule(data, { element, resolveAsset }) {
    const section = element("section", "home-schedule", {
      ariaLabel: "Home schedule",
      testId: "project-canvas-home-clock",
    });
    const image = element("img", "home-background", {
      alt: "",
      src: resolveAsset(data.background),
      testId: "project-canvas-home-clock-background",
    });
    const list = element("ul", "speech-list", {
      ariaLabel: "Clock Gloopie updates",
    });
    data.updates.forEach((update, index) => {
      list.append(
        element("li", "speech-bubble", {
          testId: `project-canvas-home-clock-status-${index + 1}`,
          text: update,
        }),
      );
    });
    section.append(image, element("span", "home-overlay"), list);
    return section;
  }

  function renderFrontYardCamera(data, { element, resolveAsset }) {
    const figure = element("figure", "camera-widget", {
      ariaLabel: "Front yard camera",
      testId: "project-canvas-front-yard-camera",
    });
    figure.append(
      element("img", "camera-image", {
        alt: "Front yard security camera view with a small parcel by the door",
        src: resolveAsset(data.image),
        testId: "project-canvas-front-yard-camera-image",
      }),
      element("span", "recording", { text: "● Recording" }),
      element("figcaption", "camera-caption", {
        text: `📦 ${data.caption}`,
      }),
    );
    return figure;
  }

  function renderFamilyLocations(data, { element }) {
    const section = element("section", "family-locations", {
      ariaLabel: "Family locations",
      testId: "project-canvas-family-locations",
    });
    const placeClasses = ["school", "cafe", "library", "work", "shops", "oboe"];
    data.places.forEach((place, index) => {
      section.append(
        element("div", `place place-${placeClasses[index]}`, {
          testId: `project-canvas-family-place-${place.toLowerCase()}`,
          text: place,
        }),
      );
    });
    section.append(
      element("div", "place place-home", {
        testId: "project-canvas-family-place-home",
        text: "⌂ Home",
      }),
      familyMember("Sally", "sally", element),
      familyMember("You", "you", element),
    );
    const dad = element("div", "dad-route");
    dad.append(
      familyMember("Dad", "dad", element),
      element("span", "dad-arrow", {
        ariaLabel: "Dad is heading toward Work",
        role: "img",
        text: "↘",
      }),
    );
    section.append(dad);
    return section;
  }

  function familyMember(name, slug, element) {
    const member = element("div", `family-member member-${slug}`, {
      ariaLabel: `${name} location`,
      role: "img",
      testId: `project-canvas-family-location-${slug}`,
    });
    member.append(
      element("span", "avatar initials", { text: name[0] }),
      document.createTextNode(name),
    );
    return member;
  }

  function renderHomeScheduleCompanion(widget, api) {
    return renderStandardGloopie(
      widget.data.gloopie,
      widget.data.gloopiePoster,
      1,
      "Home schedule helper",
      "companion home-schedule-companion",
      "project-canvas-home-schedule-gloopie-companion",
      "project-canvas-home-schedule-gloopie",
      api,
    );
  }

  function renderStandardGloopie(
    src,
    poster,
    avatarId,
    label,
    className,
    wrapperTestId,
    videoTestId,
    { element, resolveAsset },
  ) {
    const wrapper = element("div", className, { testId: wrapperTestId });
    const video = element("video", "gloopie-video", {
      ariaLabel: label,
      autoplay: "",
      loop: "",
      muted: "",
      playsinline: "",
      poster: resolveAsset(poster),
      testId: videoTestId,
    });
    video.dataset.berdAvatarId = `gloopies-${avatarId}`;
    video.muted = true;
    video.src = resolveAsset(src);
    wrapper.append(video);
    return wrapper;
  }

  function renderHenryCompanion(widget, { element, resolveAsset }) {
    const wrapper = element("div", "companion henry-companion", {
      testId: "project-canvas-chore-gloopie-companion",
    });
    const canvas = element("canvas", "henry-canvas", {
      ariaLabel: "Henry Hoover Gloopie",
      role: "img",
      testId: "project-canvas-henry-gloopie",
    });
    const video = element("video", "henry-source", {
      autoplay: "",
      loop: "",
      muted: "",
      playsinline: "",
      preload: "auto",
      src: resolveAsset(widget.data.gloopie),
      testId: "project-canvas-henry-gloopie-source",
    });
    video.muted = true;
    wrapper.append(canvas, video);
    startStackedAlphaVideo(video, canvas);
    return wrapper;
  }

  function startStackedAlphaVideo(video, canvas) {
    const maskCanvas = document.createElement("canvas");
    let frameRequest = 0;
    const paint = () => {
      if (!video.isConnected || !canvas.isConnected) return;
      const width = video.videoWidth;
      const height = Math.floor(video.videoHeight / 2);
      if (!width || !height) return;
      canvas.width = width;
      canvas.height = height;
      maskCanvas.width = width;
      maskCanvas.height = height;
      const context = canvas.getContext("2d");
      const maskContext = maskCanvas.getContext("2d");
      if (!context || !maskContext) return;
      context.drawImage(video, 0, 0, width, height, 0, 0, width, height);
      maskContext.drawImage(
        video,
        0,
        height,
        width,
        height,
        0,
        0,
        width,
        height,
      );
      const color = context.getImageData(0, 0, width, height);
      const mask = maskContext.getImageData(0, 0, width, height);
      for (let index = 3; index < color.data.length; index += 4) {
        color.data[index] = mask.data[index - 3];
      }
      context.putImageData(color, 0, 0);
    };
    const draw = () => {
      paint();
      frameRequest = window.requestAnimationFrame(draw);
    };
    video.addEventListener(
      "loadeddata",
      () => {
        window.cancelAnimationFrame(frameRequest);
        draw();
        video.play().catch(() => {});
      },
      { once: true },
    );
  }
})();
