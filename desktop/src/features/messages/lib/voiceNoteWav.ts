const DEFAULT_OUTPUT_SAMPLE_RATE = 24_000;

function writeAscii(view: DataView, offset: number, value: string) {
  for (let index = 0; index < value.length; index += 1) {
    view.setUint8(offset + index, value.charCodeAt(index));
  }
}

export function encodeVoiceNoteWav(
  channels: readonly Float32Array[],
  inputSampleRate: number,
  outputSampleRate = DEFAULT_OUTPUT_SAMPLE_RATE,
): Uint8Array {
  const inputLength = channels[0]?.length ?? 0;
  if (
    channels.length === 0 ||
    inputLength === 0 ||
    !Number.isFinite(inputSampleRate) ||
    inputSampleRate <= 0 ||
    !Number.isFinite(outputSampleRate) ||
    outputSampleRate <= 0
  ) {
    throw new Error("Cannot encode an empty voice note");
  }

  const frameCount = Math.max(
    1,
    Math.floor((inputLength * outputSampleRate) / inputSampleRate),
  );
  const bytes = new Uint8Array(44 + frameCount * 2);
  const view = new DataView(bytes.buffer);
  writeAscii(view, 0, "RIFF");
  view.setUint32(4, bytes.length - 8, true);
  writeAscii(view, 8, "WAVE");
  writeAscii(view, 12, "fmt ");
  view.setUint32(16, 16, true);
  view.setUint16(20, 1, true);
  view.setUint16(22, 1, true);
  view.setUint32(24, outputSampleRate, true);
  view.setUint32(28, outputSampleRate * 2, true);
  view.setUint16(32, 2, true);
  view.setUint16(34, 16, true);
  writeAscii(view, 36, "data");
  view.setUint32(40, frameCount * 2, true);

  const ratio = inputSampleRate / outputSampleRate;
  for (let outputIndex = 0; outputIndex < frameCount; outputIndex += 1) {
    const sourcePosition = outputIndex * ratio;
    const leftIndex = Math.min(inputLength - 1, Math.floor(sourcePosition));
    const rightIndex = Math.min(inputLength - 1, leftIndex + 1);
    const mix = sourcePosition - leftIndex;
    let sample = 0;
    for (const channel of channels) {
      const left = channel[leftIndex] ?? 0;
      const right = channel[rightIndex] ?? left;
      sample += left + (right - left) * mix;
    }
    sample = Math.max(-1, Math.min(1, sample / channels.length));
    view.setInt16(
      44 + outputIndex * 2,
      sample < 0 ? sample * 0x8000 : sample * 0x7fff,
      true,
    );
  }

  return bytes;
}
