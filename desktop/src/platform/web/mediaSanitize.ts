// Strip metadata from images in the browser so the relay's metadata-free validator accepts them
// (crates/buzz-media/src/validation.rs: PNG/JPEG/WebP/GIF allowlists). Byte-level: no re-encode, pixels are untouched.
// The caller hashes the RETURNED bytes, so sha256/size/imeta always describe what is uploaded.

const PNG_SIG = [0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a];
const PNG_KEEP = new Set(["cHRM", "gAMA", "sBIT", "sRGB", "bKGD", "hIST", "tRNS", "sPLT", "acTL", "fcTL", "fdAT"]);
const SNAPSHOT = ["buzz_agent_snapshot\0", "buzz_team_snapshot\0"];

const ascii = (b: Uint8Array, i: number, n: number) => String.fromCharCode(...b.subarray(i, i + n));
const concat = (parts: Uint8Array[]) => {
  const out = new Uint8Array(parts.reduce((n, p) => n + p.length, 0));
  let o = 0;
  for (const p of parts) (out.set(p, o), (o += p.length));
  return out;
};

function stripPng(b: Uint8Array): Uint8Array {
  const dv = new DataView(b.buffer, b.byteOffset, b.byteLength);
  const parts: Uint8Array[] = [b.subarray(0, 8)];
  let i = 8;
  let snap = false;
  while (i + 12 <= b.length) {
    const len = dv.getUint32(i);
    const end = i + 12 + len;
    if (end > b.length) return b;
    const kind = ascii(b, i + 4, 4);
    const ancillary = (b[i + 4] & 0x20) !== 0;
    let keep = !ancillary || PNG_KEEP.has(kind);
    if (kind === "tEXt") {
      const text = ascii(b, i + 8, Math.min(len, 24));
      keep = !snap && SNAPSHOT.some((k) => text.startsWith(k));
      if (keep) snap = true;
    }
    if (keep) parts.push(b.subarray(i, end));
    i = end;
    if (kind === "IEND") break; // trailing bytes after IEND are dropped too
  }
  return concat(parts);
}

function stripJpeg(b: Uint8Array): Uint8Array {
  const parts: Uint8Array[] = [b.subarray(0, 2)];
  let i = 2;
  while (i < b.length) {
    if (b[i] !== 0xff) return b;
    while (b[i + 1] === 0xff) i++;
    const m = b[i + 1];
    if (m === 0xd9) {
      parts.push(b.subarray(i, i + 2));
      return concat(parts); // anything after EOI is dropped
    }
    if (m === 0x01 || (m >= 0xd0 && m <= 0xd7)) {
      parts.push(b.subarray(i, i + 2));
      i += 2;
      continue;
    }
    const len = (b[i + 2] << 8) | b[i + 3];
    const end = i + 2 + len;
    if (len < 2 || end > b.length) return b;
    let keep = true;
    if (m === 0xe0) {
      const p = b.subarray(i + 4, end);
      keep = p.length >= 14 && ascii(p, 0, 5) === "JFIF\0" && p.length === 14 + 3 * p[12] * p[13];
    } else if (m === 0xee) {
      keep = len - 2 === 12 && ascii(b, i + 4, 5) === "Adobe";
    } else if ((m >= 0xe1 && m <= 0xed) || m === 0xef || m === 0xfe) keep = false;
    if (keep) parts.push(b.subarray(i, end));
    if (m === 0xda) {
      // entropy-coded scan: copy up to the next real marker (not stuffed 0xFF00 / RSTn)
      let j = end;
      while (j + 1 < b.length && !(b[j] === 0xff && b[j + 1] !== 0x00 && !(b[j + 1] >= 0xd0 && b[j + 1] <= 0xd7))) j++;
      parts.push(b.subarray(end, j));
      i = j;
    } else i = end;
  }
  return concat(parts);
}

function stripWebp(b: Uint8Array): Uint8Array {
  const dv = new DataView(b.buffer, b.byteOffset, b.byteLength);
  const keepKinds = new Set(["VP8 ", "VP8L", "VP8X", "ALPH", "ANIM", "ANMF"]);
  const chunks: Uint8Array[] = [];
  let i = 12;
  while (i + 8 <= b.length) {
    const kind = ascii(b, i, 4);
    const len = dv.getUint32(i + 4, true);
    const end = i + 8 + len + (len & 1);
    if (end > b.length + 1) return b;
    if (keepKinds.has(kind)) {
      const c = b.slice(i, Math.min(end, b.length));
      if (kind === "VP8X") c[8] &= ~(0x20 | 0x08 | 0x04); // ICC, EXIF, XMP flags
      chunks.push(c);
    }
    i = end;
  }
  const body = concat(chunks);
  const out = new Uint8Array(12 + body.length);
  out.set(b.subarray(0, 12));
  out.set(body, 12);
  new DataView(out.buffer).setUint32(4, out.length - 8, true);
  return out;
}

function stripGif(b: Uint8Array): Uint8Array {
  const parts: Uint8Array[] = [];
  const subBlocks = (i: number) => {
    for (;;) {
      if (i >= b.length) return -1;
      const n = b[i];
      i += 1 + n;
      if (n === 0) return i;
    }
  };
  let i = 13;
  if (b[10] & 0x80) i += 3 << ((b[10] & 7) + 1);
  parts.push(b.subarray(0, i));
  while (i < b.length) {
    const t = b[i];
    if (t === 0x3b) {
      parts.push(b.subarray(i, i + 1));
      return concat(parts);
    }
    if (t === 0x2c) {
      let j = i + 10;
      if (b[i + 9] & 0x80) j += 3 << ((b[i + 9] & 7) + 1);
      const end = subBlocks(j + 1);
      if (end < 0) return b;
      parts.push(b.subarray(i, end));
      i = end;
    } else if (t === 0x21) {
      const label = b[i + 1];
      const end = subBlocks(i + 2);
      if (end < 0) return b;
      const app = label === 0xff ? ascii(b, i + 3, 11) : "";
      if (label === 0xf9 || app === "NETSCAPE2.0" || app === "ANIMEXTS1.0") parts.push(b.subarray(i, end));
      i = end;
    } else return b;
  }
  return b;
}

/** Returns the bytes with all forbidden metadata channels removed (same bytes if already clean or unrecognized). */
export function stripImageMetadata(bytes: Uint8Array): Uint8Array {
  try {
    const b = bytes;
    if (b.length > 8 && PNG_SIG.every((v, k) => b[k] === v)) return stripPng(b);
    if (b[0] === 0xff && b[1] === 0xd8) return stripJpeg(b);
    if (b.length > 12 && ascii(b, 0, 4) === "RIFF" && ascii(b, 8, 4) === "WEBP") return stripWebp(b);
    if (b.length > 13 && ascii(b, 0, 4) === "GIF8") return stripGif(b);
  } catch {
    /* malformed: let the relay report it */
  }
  return bytes;
}
