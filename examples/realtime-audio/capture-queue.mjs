// Bound latency/memory, not scheduler jitter. Never drop or reorder microphone samples.
export class CaptureQueue {
  constructor() {
    this.frames = [];
    this.samples = 0;
  }
  push(pcm) {
    if (this.samples + pcm.length > 120000)
      throw Error("microphone transport stalled for over five seconds");
    this.frames.push(pcm);
    this.samples += pcm.length;
  }
  shift() {
    const count = Math.min(this.samples, 2400),
      out = new Int16Array(count);
    let used = 0;
    while (used < count) {
      const head = this.frames[0],
        n = Math.min(head.length, count - used);
      out.set(head.subarray(0, n), used);
      used += n;
      if (n === head.length) this.frames.shift();
      else this.frames[0] = head.subarray(n);
    }
    this.samples -= count;
    return out;
  }
  get length() {
    return this.samples;
  }
}
