const hero = document.querySelector("#hero"),
  ctx = hero.getContext("2d"),
  small = document.querySelector("#small"),
  sctx = small.getContext("2d");
const path =
  "M128 70.3 A91.7 91.7 0 1 0 128 238.7 Z M338 70.3 A91.7 91.7 0 1 1 338 238.7 Z M162 0 H304 A34 34 0 0 1 338 34 V275 A34 34 0 0 1 304 309 H162 A34 34 0 0 1 128 275 V34 A34 34 0 0 1 162 0 Z M220.3 84.4 A27 27 0 1 1 166.3 84.4 A27 27 0 1 1 220.3 84.4 Z M303 84.4 A27 27 0 1 1 249 84.4 A27 27 0 1 1 303 84.4 Z M171.3 157.2 H298.2 A5 5 0 0 1 303.2 162.2 V190.5 A5 5 0 0 1 298.2 195.5 H171.3 A5 5 0 0 1 166.3 190.5 V162.2 A5 5 0 0 1 171.3 157.2 Z M171.9 235.1 H298.1 A5 5 0 0 1 303.1 240.1 V267.7 A5 5 0 0 1 298.1 272.7 H171.9 A5 5 0 0 1 166.9 267.7 V240.1 A5 5 0 0 1 171.9 235.1 Z";
const bee = new Path2D(path),
  mark = new Image();
mark.src = "buzz-wordmark.png";
const logo = document.createElement("canvas");
logo.width = 640;
logo.height = Math.round((640 * 326) / 777);
const gl = logo.getContext("webgl", {
  alpha: true,
  premultipliedAlpha: true,
  preserveDrawingBuffer: true,
});
let program, phase;
async function setupLogo() {
  await mark.decode();
  if (!gl) return;
  const fragment = await (await fetch("wordmark.frag")).text();
  const compile = (type, source) => {
    const shader = gl.createShader(type);
    gl.shaderSource(shader, source);
    gl.compileShader(shader);
    if (!gl.getShaderParameter(shader, gl.COMPILE_STATUS))
      throw Error(gl.getShaderInfoLog(shader));
    return shader;
  };
  program = gl.createProgram();
  gl.attachShader(
    program,
    compile(
      gl.VERTEX_SHADER,
      "attribute vec2 position; void main(){gl_Position=vec4(position,0.0,1.0);}",
    ),
  );
  gl.attachShader(program, compile(gl.FRAGMENT_SHADER, fragment));
  gl.linkProgram(program);
  if (!gl.getProgramParameter(program, gl.LINK_STATUS))
    throw Error(gl.getProgramInfoLog(program));
  gl.useProgram(program);
  gl.bindBuffer(gl.ARRAY_BUFFER, gl.createBuffer());
  gl.bufferData(
    gl.ARRAY_BUFFER,
    new Float32Array([-1, -1, 1, -1, -1, 1, 1, 1]),
    gl.STATIC_DRAW,
  );
  const pos = gl.getAttribLocation(program, "position");
  gl.enableVertexAttribArray(pos);
  gl.vertexAttribPointer(pos, 2, gl.FLOAT, false, 0, 0);
  gl.bindTexture(gl.TEXTURE_2D, gl.createTexture());
  gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_MIN_FILTER, gl.LINEAR);
  gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_MAG_FILTER, gl.LINEAR);
  gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_WRAP_S, gl.CLAMP_TO_EDGE);
  gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_WRAP_T, gl.CLAMP_TO_EDGE);
  const source = document.createElement("canvas");
  source.width = mark.naturalWidth;
  source.height = mark.naturalHeight;
  source.getContext("2d").drawImage(mark, 0, 0);
  gl.pixelStorei(gl.UNPACK_ALIGNMENT, 1);
  gl.texImage2D(gl.TEXTURE_2D, 0, gl.RGBA, gl.RGBA, gl.UNSIGNED_BYTE, source);
  gl.viewport(0, 0, logo.width, logo.height);
  gl.uniform2f(
    gl.getUniformLocation(program, "uSize"),
    logo.width,
    logo.height,
  );
  gl.uniform4f(
    gl.getUniformLocation(program, "uColor"),
    35 / 255,
    30 / 255,
    30 / 255,
    1,
  );
  phase = gl.getUniformLocation(program, "uPhase");
}

let posterTime = Number(localStorage.getItem("buzz-invite-poster-time") ?? "0");
if (!Number.isFinite(posterTime) || posterTime < 0 || posterTime >= 16)
  posterTime = 0;
let t = 0,
  playing = !matchMedia("(prefers-reduced-motion: reduce)").matches,
  poster = false,
  last = performance.now();
function fly(time, start, direction, y, pair = false, endY = y) {
  const phase = (time - start + 16) % 16;
  if (phase > 2.6) return;
  const u = phase / 2.6,
    x = direction > 0 ? -90 + 1140 * u : 1050 - 1140 * u;
  const yy =
    y +
    (endY - y) * u +
    45 * Math.sin(u * Math.PI * 2) +
    16 * Math.sin(u * Math.PI * 4);
  const dx = direction * 1140,
    dy =
      endY -
      y +
      90 * Math.PI * Math.cos(u * Math.PI * 2) +
      64 * Math.PI * Math.cos(u * Math.PI * 4);
  ctx.save();
  ctx.translate(x, yy);
  ctx.rotate(Math.atan2(dy, dx) + Math.PI / 2);
  const scale = (pair ? 60 : 73) / 466;
  ctx.scale(scale * (1 + 0.06 * Math.sin(time * Math.PI * 20)), scale);
  ctx.translate(-233, -154.5);
  ctx.fillStyle = pair || start === 8.4 ? "#dbfa52" : "#d7e7f5";
  ctx.fill(bee, "evenodd");
  ctx.restore();
}
// Arc-length sampling keeps the solo bee's speed steady through the loop.
const loopPath = document.createElementNS("http://www.w3.org/2000/svg", "path");
loopPath.setAttribute(
  "d",
  "M -90 150 C 100 150 220 100 350 100 C 500 100 510 255 400 250 C 300 245 320 110 450 110 C 650 110 800 180 1050 180",
);
const loopLength = loopPath.getTotalLength();
function loopingBee(time) {
  if (time > 3.6) return;
  const distance = (time / 3.6) * loopLength;
  const p = loopPath.getPointAtLength(distance),
    a = loopPath.getPointAtLength(Math.max(0, distance - 1)),
    b = loopPath.getPointAtLength(Math.min(loopLength, distance + 1));
  ctx.save();
  ctx.translate(p.x, p.y);
  ctx.rotate(Math.atan2(b.y - a.y, b.x - a.x) + Math.PI / 2);
  const scale = 73 / 466;
  ctx.scale(scale * (1 + 0.06 * Math.sin(time * Math.PI * 20)), scale);
  ctx.translate(-233, -154.5);
  ctx.fillStyle = "#d7e7f5";
  ctx.fill(bee, "evenodd");
  ctx.restore();
}
window.renderAt = (time) => {
  const q = ((time % 16) + 16) % 16;
  ctx.fillStyle = "#d7d72e";
  ctx.fillRect(0, 0, 960, 540);
  if (gl && program) {
    gl.uniform1f(phase, (q % 0.4) / 0.4);
    gl.drawArrays(gl.TRIANGLE_STRIP, 0, 4);
    ctx.drawImage(logo, 160, (540 - logo.height) / 2);
  } else {
    ctx.drawImage(mark, 160, (540 - logo.height) / 2, 640, logo.height);
  }
  loopingBee(q);
  fly(q, 4.6, -1, 460, false, 90);
  fly(q, 4.8, -1, 505, true, 160);
  fly(q, 8.4, 1, 70, false, 480);
  fly(q, 12, -1, 120, false, 360);
  fly(q, 12.2, -1, 165, true, 435);
  sctx.drawImage(hero, 0, 0);
};
function state() {
  document.querySelector("#play").textContent = playing ? "Pause" : "Play";
  document.querySelector("#poster").setAttribute("aria-pressed", poster);
}
document.querySelector("#play").onclick = () => {
  playing = !playing;
  poster = false;
  state();
};
document.querySelector("#poster").onclick = () => {
  poster = !poster;
  if (poster) {
    playing = false;
    t = posterTime;
    renderAt(t);
  }
  state();
};
document.querySelector("#seek").oninput = (e) => {
  t = Number(e.target.value);
  playing = false;
  poster = false;
  renderAt(t);
  state();
};
function savePoster(time) {
  posterTime = Math.round(time * 30) / 30;
  if (posterTime >= 16) posterTime = 0;
  renderAt(posterTime);
  const data = hero.toDataURL("image/png");
  document.querySelector("#poster-image").src = data;
  document.querySelector("#download-poster").href = data;
  document.querySelector("#poster-status").textContent =
    `Saved poster: ${posterTime.toFixed(2)}s · 960 × 540`;
  localStorage.setItem("buzz-invite-poster-time", String(posterTime));
}
document.querySelector("#save-poster").onclick = () => {
  playing = false;
  poster = true;
  savePoster(t);
  t = posterTime;
  state();
};
function posterChoices() {
  for (const [time, label] of [
    [0, "Wordmark only"],
    [1, "Solo bee"],
    [5.9, "Pair crossing"],
    [13.1, "Diagonal pair"],
  ]) {
    renderAt(time);
    const button = document.createElement("button");
    button.style.padding = "5px";
    button.innerHTML =
      '<img alt="" style="width:100%;border-radius:5px" src="' +
      hero.toDataURL("image/png") +
      '"><span style="font-size:11px">' +
      label +
      "</span>";
    button.onclick = () => {
      t = time;
      playing = false;
      poster = false;
      renderAt(t);
      state();
    };
    document.querySelector("#choices").append(button);
  }
  savePoster(posterTime);
}
setupLogo()
  .then(() => {
    posterChoices();
    window.ready = true;
    renderAt(0);
    state();
    requestAnimationFrame(function frame(now) {
      if (playing && !window.captureMode) t = (t + (now - last) / 1000) % 16;
      last = now;
      if (!window.captureMode) renderAt(t);
      document.querySelector("#seek").value = t;
      document.querySelector("#time").textContent = `${t.toFixed(1)} / 16s`;
      requestAnimationFrame(frame);
    });
  })
  .catch((error) => {
    console.error(error);
    document.querySelector(".caption")?.remove();
  });
