import { invokeTauri } from "@/shared/api/tauri";

export type SpotifyStatus = {
  configured: boolean;
  connected: boolean;
  connectedAt: number | null;
  scopes: string[];
};

export type SpotifyDevice = {
  id: string | null;
  name: string;
  deviceType: string;
  isActive: boolean;
  isRestricted: boolean;
  volumePercent: number | null;
};

export type SpotifyPlaybackInput = {
  contextUri?: string;
  deviceId?: string;
  positionMs?: number;
  uris?: string[];
};

export type SpotifyPlaybackState = {
  contextUri: string | null;
  device: SpotifyDevice | null;
  isPlaying: boolean;
  item: SpotifyPlaybackItem | null;
  progressMs: number | null;
  timestamp: number | null;
};

export type SpotifyPlaybackItem = {
  artists: string[];
  durationMs: number | null;
  imageUrl: string | null;
  itemType: string | null;
  name: string;
  uri: string;
};

export type SpotifyDeviceInput = {
  deviceId?: string;
};

export type SpotifySeekInput = SpotifyDeviceInput & {
  positionMs: number;
};

type RawSpotifyStatus = {
  configured: boolean;
  connected: boolean;
  connected_at: number | null;
  scopes: string[];
};

type RawSpotifyDevice = {
  id: string | null;
  name: string;
  device_type: string;
  is_active: boolean;
  is_restricted: boolean;
  volume_percent: number | null;
};

type RawSpotifyPlaybackState = {
  context_uri: string | null;
  device: RawSpotifyDevice | null;
  is_playing: boolean;
  item: RawSpotifyPlaybackItem | null;
  progress_ms: number | null;
  timestamp: number | null;
};

type RawSpotifyPlaybackItem = {
  artists: string[];
  duration_ms: number | null;
  image_url: string | null;
  item_type: string | null;
  name: string;
  uri: string;
};

function fromRawStatus(raw: RawSpotifyStatus): SpotifyStatus {
  return {
    configured: raw.configured,
    connected: raw.connected,
    connectedAt: raw.connected_at,
    scopes: raw.scopes,
  };
}

function fromRawDevice(raw: RawSpotifyDevice): SpotifyDevice {
  return {
    id: raw.id,
    name: raw.name,
    deviceType: raw.device_type,
    isActive: raw.is_active,
    isRestricted: raw.is_restricted,
    volumePercent: raw.volume_percent,
  };
}

function fromRawPlaybackItem(raw: RawSpotifyPlaybackItem): SpotifyPlaybackItem {
  return {
    artists: raw.artists,
    durationMs: raw.duration_ms,
    imageUrl: raw.image_url,
    itemType: raw.item_type,
    name: raw.name,
    uri: raw.uri,
  };
}

function fromRawPlaybackState(
  raw: RawSpotifyPlaybackState,
): SpotifyPlaybackState {
  return {
    contextUri: raw.context_uri,
    device: raw.device ? fromRawDevice(raw.device) : null,
    isPlaying: raw.is_playing,
    item: raw.item ? fromRawPlaybackItem(raw.item) : null,
    progressMs: raw.progress_ms,
    timestamp: raw.timestamp,
  };
}

export async function getSpotifyStatus(): Promise<SpotifyStatus> {
  return fromRawStatus(
    await invokeTauri<RawSpotifyStatus>("get_spotify_status"),
  );
}

export async function connectSpotify(): Promise<SpotifyStatus> {
  return fromRawStatus(await invokeTauri<RawSpotifyStatus>("connect_spotify"));
}

export async function disconnectSpotify(): Promise<SpotifyStatus> {
  return fromRawStatus(
    await invokeTauri<RawSpotifyStatus>("disconnect_spotify"),
  );
}

export async function getSpotifyDevices(): Promise<SpotifyDevice[]> {
  const raw = await invokeTauri<RawSpotifyDevice[]>("get_spotify_devices");
  return raw.map(fromRawDevice);
}

export async function getSpotifyPlaybackState(): Promise<SpotifyPlaybackState | null> {
  const raw = await invokeTauri<RawSpotifyPlaybackState | null>(
    "get_spotify_playback_state",
  );
  return raw ? fromRawPlaybackState(raw) : null;
}

export async function startSpotifyPlayback(
  input: SpotifyPlaybackInput = {},
): Promise<void> {
  await invokeTauri("start_spotify_playback", { input });
}

export async function pauseSpotifyPlayback(
  input: SpotifyDeviceInput = {},
): Promise<void> {
  await invokeTauri("pause_spotify_playback", { input });
}

export async function skipSpotifyNext(
  input: SpotifyDeviceInput = {},
): Promise<void> {
  await invokeTauri("skip_spotify_next", { input });
}

export async function skipSpotifyPrevious(
  input: SpotifyDeviceInput = {},
): Promise<void> {
  await invokeTauri("skip_spotify_previous", { input });
}

export async function seekSpotifyPlayback(
  input: SpotifySeekInput,
): Promise<void> {
  await invokeTauri("seek_spotify_playback", { input });
}
