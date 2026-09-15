const $ = (id) => document.getElementById(id),
  st = $("st"),
  log = $("log");
let turns = 0,
  micAnalyser,
  voiceAnalyser,
  connected = false;
const phases = {
  LISTEN: ["Listening.", "TAKE YOUR TIME", "listen"],
  SPEC: ["Thinking.", "FOLLOWING YOUR THOUGHT", "think"],
  REPLY: ["Speaking.", "A THOUGHT TAKES VOICE", "speak"],
  PLAY: ["Speaking.", "FINISHING THE THOUGHT", "speak"],
  OFF: ["Ready.", "YOUR VOICE STARTS HERE", ""],
};
function setState(state) {
  // Audio arrives in many chunks; announce only state transitions.
  if (document.body.dataset.state === state) return;
  const v = phases[state] || phases.OFF;
  document.body.dataset.state = state;
  st.textContent = v[0];
  $("state-sub").textContent = v[1];
  for (const p of ["listen", "think", "speak"])
    $(`phase-${p}`).classList.toggle("active", p === v[2]);
  $("announcement").textContent = `Frankie: ${v[0]}`;
  $("voice-label").textContent = v[2] === "speak" ? "OUTPUT" : "STANDBY";
}
function clearEmpty() {
  $("empty")?.remove();
}
function follow() {
  log.scrollTop = log.scrollHeight;
}
function entry(who, text) {
  const d = document.createElement("div");
  d.className = `entry ${who === "You" ? "user" : ""}`;
  const l = document.createElement("div");
  l.className = `label ${who === "Frankie" ? "frankie" : ""}`;
  l.textContent = who;
  const p = document.createElement("p");
  p.textContent = text;
  d.append(l, p);
  return d;
}
function newTurn() {
  clearEmpty();
  const d = document.createElement("article");
  d.className = "turn";
  log.append(d);
  while (log.children.length > 60) log.firstElementChild.remove();
  return d;
}
function note(text) {
  clearEmpty();
  const d = document.createElement("div");
  d.className = "note";
  d.textContent = text;
  log.append(d);
  while (log.children.length > 60) log.firstElementChild.remove();
  follow();
}

$("fullscreen").onclick = async () => {
  try {
    if (document.fullscreenElement) await document.exitFullscreen();
    else if (document.documentElement.requestFullscreen)
      await document.documentElement.requestFullscreen();
    else
      $("help").textContent =
        "Use your browser’s fullscreen command for browser fullscreen.";
  } catch {
    $("help").textContent =
      "Use your browser’s fullscreen command for browser fullscreen.";
  }
};

const reduced = matchMedia("(prefers-reduced-motion: reduce)"),
  canvases = ["scope", "mic-meter", "voice-meter"].map((id) => $(id)),
  pens = canvases.map((c) => c.getContext("2d")),
  micData = new Uint8Array(256).fill(128),
  voiceData = new Uint8Array(256).fill(128);
let micLevel = 0,
  voiceLevel = 0;
function resize() {
  const d = Math.min(devicePixelRatio || 1, 2);
  canvases.forEach((c, i) => {
    const b = c.getBoundingClientRect();
    c.width = Math.round(b.width * d);
    c.height = Math.round(b.height * d);
    pens[i].setTransform(d, 0, 0, d, 0, 0);
  });
}
new ResizeObserver(resize).observe($("scope").parentElement);
window.addEventListener("resize", resize);
resize();
function level(analyser, data) {
  if (!analyser) return 0;
  analyser.getByteTimeDomainData(data);
  let v = 0;
  for (const x of data) v += ((x - 128) / 128) ** 2;
  return Math.min(1, Math.sqrt(v / data.length) * 5);
}
function draw(now) {
  if (!document.hidden) {
    micLevel = 0.7 * micLevel + 0.3 * level(micAnalyser, micData);
    voiceLevel = 0.7 * voiceLevel + 0.3 * level(voiceAnalyser, voiceData);
    const c = canvases[0],
      p = pens[0],
      w = c.clientWidth,
      h = c.clientHeight,
      x = w / 2,
      y = h / 2,
      r = Math.min(h * 0.3, w * 0.31),
      t = reduced.matches ? 0 : now * 0.0002;
    p.clearRect(0, 0, w, h);
    p.strokeStyle = "#34505c";
    p.lineWidth = 0.6;
    p.beginPath();
    p.moveTo(20, y);
    p.lineTo(x - r - 18, y);
    p.moveTo(x + r + 18, y);
    p.lineTo(w - 20, y);
    p.moveTo(x, 10);
    p.lineTo(x, y - r - 12);
    p.moveTo(x, y + r + 12);
    p.lineTo(x, h - 10);
    p.stroke();
    for (let k = 0; k < 120; k++) {
      const a = (k * Math.PI) / 60 - Math.PI / 2,
        major = k % 10 === 0;
      const amp = voiceLevel || micLevel;
      const len = major
        ? 11
        : 4 + amp * 12 * (0.5 + 0.5 * Math.sin(k * 0.8 + t * 20));
      p.strokeStyle = k < Math.round(amp * 120) ? "#72ebdf" : "#34505c";
      p.lineWidth = major ? 1.5 : 1;
      p.beginPath();
      p.moveTo(x + Math.cos(a) * r, y + Math.sin(a) * r);
      p.lineTo(x + Math.cos(a) * (r + len), y + Math.sin(a) * (r + len));
      p.stroke();
    }
    p.strokeStyle = "#65cfc8";
    p.lineWidth = 1;
    p.beginPath();
    for (let k = 0; k <= 180; k++) {
      const a = (k * Math.PI) / 90;
      const data = voiceLevel > 0.02 ? voiceData : micData;
      const v = (data[Math.floor((k * 255) / 180)] - 128) / 128;
      const rr = r - 28 + (connected ? v * 35 : 0);
      const xx = x + Math.cos(a) * rr,
        yy = y + Math.sin(a) * rr;
      if (k === 0) p.moveTo(xx, yy);
      else p.lineTo(xx, yy);
    }
    p.stroke();
    p.strokeStyle = "#315762";
    p.lineWidth = 0.5;
    p.beginPath();
    p.arc(x, y, r - 13, 0, Math.PI * 2);
    p.stroke();
    p.strokeStyle = "#72ebdf";
    p.lineWidth = 2;
    p.beginPath();
    p.arc(x, y, r + 20, t, t + 0.6 + voiceLevel * 2);
    p.stroke();
    p.strokeStyle = "#a2e8eb";
    p.lineWidth = 1;
    p.beginPath();
    p.arc(x, y, r - 19, -t + Math.PI, -t + Math.PI + 0.35);
    p.stroke();
    [micLevel, voiceLevel].forEach((v, j) => {
      const cc = canvases[j + 1],
        q = pens[j + 1],
        ww = cc.clientWidth,
        hh = cc.clientHeight,
        n = 42;
      q.clearRect(0, 0, ww, hh);
      for (let i = 0; i < n; i++) {
        q.fillStyle = i < Math.round(v * n) ? "#72ebdf" : "#233943";
        q.fillRect(
          (i * ww) / n,
          hh - 6 - (i / n) * 14,
          Math.max(1, ww / n - 3),
          6 + (i / n) * 14,
        );
      }
    });
  }
  requestAnimationFrame(draw);
}
requestAnimationFrame(draw);

export function state(s) {
  setState(s);
}
export function analyzers(mic, voice) {
  micAnalyser = mic;
  voiceAnalyser = voice;
}
export function connection(value, message) {
  if (value && !connected) {
    // Provider item IDs restart with each fresh agent session.
    log.replaceChildren();
    turns = 0;
    $("turn-count").textContent = "00 TURNS";
    $("latency").textContent = "FIRST SOUND —";
  }
  connected = value;
  document.body.dataset.connected = String(value);
  $("connection").textContent = value ? "CONNECTED" : "OFFLINE";
  $("signal").textContent = value
    ? "BUZZ AGENT → LLAMA.CPP"
    : "AWAITING CONNECTION";
  $("mic-label").textContent = value ? "LIVE · 24 kHz" : "STANDBY";
  $("go-label").textContent = value ? "End conversation" : "Start talking";
  $("go").disabled = false;
  $("help").textContent = message;
  $("instruction").textContent = value
    ? "Ready to listen."
    : "Make yourself heard.";
  if (!value) setState("OFF");
}
export function transcript(text) {
  const row = newTurn();
  row.append(entry("Frankie", text));
  turns++;
  $("turn-count").textContent = `${String(turns).padStart(2, "0")} TURNS`;
  follow();
}
export { note };

export function userTranscript(text, itemId) {
  let row = [...log.children].find((r) => r.dataset.inputId === itemId);
  if (!row) {
    row = newTurn();
    row.dataset.inputId = itemId;
  }
  row.replaceChildren(entry("You", text || "[No speech recognized]"));
  follow();
}
export function removeInput(itemId) {
  [...log.children].find((r) => r.dataset.inputId === itemId)?.remove();
}
