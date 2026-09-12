// Both directions share the device sample clock, not message-arrival timestamps.
class DuplexAudio extends AudioWorkletProcessor {
  constructor() {
    super();
    this.input = new Int16Array(480);
    this.systemInput = new Int16Array(480);
    this.inputUsed = 0;
    this.capture = false;
    this.queue = [];
    this.queued = 0;
    this.played = 0;
    this.item = null;
    this.blocked = new Set();
    this.aside = [];
    this.asideQueued = 0;
    this.asideId = null;
    this.captured = 0;
    this.rendered = 0;
    this.startedAt = null;
    this.gapFrame = null;
    this.tick = 0;
    this.reportedItem = null;
    this.reportedSamples = 0;
    this.port.onmessage = ({ data }) => {
      if (data.type === "capture") this.capture = data.enabled;
      if (data.type === "backchannel") {
        if (this.queued || this.blocked.has(data.id)) return;
        if (this.asideId !== data.id) {
          if (this.asideQueued) {
            this.fail("overlapping backchannels");
            return;
          }
          this.asideId = data.id;
        }
        if (this.asideQueued + data.pcm.length > 30720) {
          this.fail("backchannel buffer exceeded 1.28 seconds");
          return;
        }
        this.aside.push({ pcm: data.pcm, offset: 0 });
        this.asideQueued += data.pcm.length;
      }
      if (data.type === "audio") {
        if (this.asideId) this.blocked.add(this.asideId);
        this.aside = [];
        this.asideQueued = 0;
        if (this.blocked.has(data.itemId)) return;
        if (this.item && this.item.itemId !== data.itemId && this.queued) {
          this.fail("overlapping playback items");
          return;
        }
        if (!this.item || this.item.itemId !== data.itemId) {
          this.item = data;
          this.played = 0;
          this.startedAt = null;
          this.gapFrame = null;
        }
        // Bound queued playback independently of response length; native speech is paced.
        if (this.queued + data.pcm.length > 24000 * 30) {
          this.fail("playback buffer exceeded thirty seconds");
          return;
        }
        this.queue.push({ pcm: data.pcm, offset: 0 });
        this.queued += data.pcm.length;
      }
      if (data.type === "clear") {
        if (this.item) this.blocked.add(this.item.itemId);
        if (this.blocked.size > 4096) {
          this.fail("playback item limit");
          return;
        }
        this.queue = [];
        this.queued = 0;
        if (this.asideId) this.blocked.add(this.asideId);
        this.aside = [];
        this.asideQueued = 0;
        this.position("stopped", data.requestId);
        this.item = null;
        this.gapFrame = null;
      }
    };
  }
  fail(message) {
    this.queue = [];
    this.queued = 0;
    this.aside = [];
    this.asideQueued = 0;
    this.capture = false;
    this.port.postMessage({ type: "error", message });
  }
  position(type, requestId) {
    if (
      type === "position" &&
      (!this.item ||
        (this.reportedItem === this.item.itemId &&
          this.reportedSamples === this.played))
    )
      return;
    this.reportedItem = this.item?.itemId;
    this.reportedSamples = this.played;
    this.port.postMessage({
      type,
      requestId,
      startedAt: this.startedAt,
      playback: this.item
        ? {
            responseId: this.item.responseId,
            itemId: this.item.itemId,
            contentIndex: 0,
            playedSamples: this.played,
          }
        : null,
    });
  }
  process(inputs, outputs) {
    if (sampleRate !== 24000) {
      this.fail("device context must run at 24 kHz");
      return false;
    }
    const input = inputs[0]?.[0],
      output = outputs[0][0];
    const frame =
      typeof currentFrame === "number" ? currentFrame : this.rendered;
    for (let i = 0; i < output.length; i++) {
      const head = this.queue[0];
      if (head) {
        if (this.gapFrame !== null) {
          this.port.postMessage({
            type: "playback_gap",
            itemId: this.item.itemId,
            playedSamples: this.played,
            missingSamples: frame + i - this.gapFrame,
          });
          this.gapFrame = null;
        }
        if (this.startedAt === null) this.startedAt = (frame + i) / sampleRate;
        output[i] = head.pcm[head.offset++] / 32768;
        this.played++;
        this.queued--;
        if (head.offset === head.pcm.length) this.queue.shift();
      } else {
        // Count a gap only if more audio resumes in this same item; idle time is not an underrun.
        if (this.item && this.played && this.gapFrame === null)
          this.gapFrame = frame + i;
        const aside = this.aside[0];
        if (aside) {
          output[i] = aside.pcm[aside.offset++] / 32768;
          this.asideQueued--;
          if (aside.offset === aside.pcm.length) this.aside.shift();
        } else output[i] = 0;
      }
      if (this.capture) {
        this.captured++;
        this.systemInput[this.inputUsed] = Math.round(output[i] * 32768);
        this.input[this.inputUsed++] = Math.round(
          Math.max(-1, Math.min(1, input?.[i] || 0)) * 32767,
        );
        if (this.inputUsed === 480) {
          const pcm = this.input,
            playback = this.systemInput;
          this.port.postMessage(
            {
              type: "capture",
              pcm,
              playback,
              captureSamples: this.captured,
              captureTime: (frame + i + 1) / sampleRate,
            },
            [pcm.buffer, playback.buffer],
          );
          this.input = new Int16Array(480);
          this.systemInput = new Int16Array(480);
          this.inputUsed = 0;
        }
      }
    }
    this.rendered += output.length;
    this.tick += output.length;
    if (this.tick >= 2400) {
      this.tick = 0;
      this.position("position");
    }
    return true;
  }
}
registerProcessor("duplex-audio", DuplexAudio);
