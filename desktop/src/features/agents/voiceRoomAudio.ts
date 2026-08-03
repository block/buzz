type VoiceRoomParticipant = {
  destination: MediaStreamAudioDestinationNode;
  remoteSource: MediaStreamAudioSourceNode | null;
};

export function mixMinusRecipients(
  participantIds: readonly string[],
  speakerId: string,
): string[] {
  return participantIds.filter((participantId) => participantId !== speakerId);
}

class VoiceRoomAudioRouter {
  private context: AudioContext | null = null;
  private microphoneStream: MediaStream | null = null;
  private microphoneSource: MediaStreamAudioSourceNode | null = null;
  private initialization: Promise<void> | null = null;
  private readonly participants = new Map<string, VoiceRoomParticipant>();

  async join(participantId: string): Promise<MediaStream> {
    await this.ensureAudioGraph();
    const existing = this.participants.get(participantId);
    if (existing) return existing.destination.stream;

    const context = this.context;
    const microphoneSource = this.microphoneSource;
    if (!context || !microphoneSource) {
      throw new Error("Buzz could not initialize the shared voice room.");
    }
    if (context.state === "suspended") await context.resume();

    const destination = context.createMediaStreamDestination();
    microphoneSource.connect(destination);
    for (const [speakerId, participant] of this.participants) {
      if (speakerId !== participantId) {
        participant.remoteSource?.connect(destination);
      }
    }
    this.participants.set(participantId, {
      destination,
      remoteSource: null,
    });
    return destination.stream;
  }

  setRemoteStream(participantId: string, stream: MediaStream) {
    const participant = this.participants.get(participantId);
    const context = this.context;
    if (!participant || !context || stream.getAudioTracks().length === 0) {
      return;
    }

    participant.remoteSource?.disconnect();
    const source = context.createMediaStreamSource(stream);
    participant.remoteSource = source;
    for (const recipientId of mixMinusRecipients(
      [...this.participants.keys()],
      participantId,
    )) {
      const recipient = this.participants.get(recipientId);
      if (recipient) source.connect(recipient.destination);
    }
  }

  setMuted(participantId: string, muted: boolean) {
    const participant = this.participants.get(participantId);
    for (const track of participant?.destination.stream.getAudioTracks() ??
      []) {
      track.enabled = !muted;
    }
  }

  leave(participantId: string) {
    const participant = this.participants.get(participantId);
    if (!participant) return;

    participant.remoteSource?.disconnect();
    this.microphoneSource?.disconnect(participant.destination);
    for (const other of this.participants.values()) {
      if (other === participant) continue;
      try {
        other.remoteSource?.disconnect(participant.destination);
      } catch {
        // The source may not have been connected to a participant that joined later.
      }
    }
    for (const track of participant.destination.stream.getTracks())
      track.stop();
    this.participants.delete(participantId);

    if (this.participants.size === 0) {
      this.microphoneSource?.disconnect();
      for (const track of this.microphoneStream?.getTracks() ?? [])
        track.stop();
      this.microphoneSource = null;
      this.microphoneStream = null;
      const context = this.context;
      this.context = null;
      if (context) void context.close();
    }
  }

  private async ensureAudioGraph() {
    if (this.context && this.microphoneStream && this.microphoneSource) return;
    if (this.initialization) {
      await this.initialization;
      return;
    }

    this.initialization = (async () => {
      const stream = await navigator.mediaDevices.getUserMedia({
        audio: {
          echoCancellation: true,
          noiseSuppression: true,
          autoGainControl: true,
        },
      });
      const context = new AudioContext();
      this.microphoneStream = stream;
      this.context = context;
      this.microphoneSource = context.createMediaStreamSource(stream);
    })();
    try {
      await this.initialization;
    } finally {
      this.initialization = null;
    }
  }
}

export const voiceRoomAudio = new VoiceRoomAudioRouter();
