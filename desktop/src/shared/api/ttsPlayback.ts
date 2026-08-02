import { invokeTauri } from "@/shared/api/tauri";

export const getTtsPlaybackSpeed = () =>
  invokeTauri<number>("get_tts_playback_speed");

export const setTtsPlaybackSpeed = (speed: number) =>
  invokeTauri<void>("set_tts_playback_speed", { speed });
