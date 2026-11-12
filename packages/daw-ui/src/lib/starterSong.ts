import { invoke } from '@tauri-apps/api/core'
import { useTempoMapStore } from '../stores/tempoMapStore'

/** What the engine says about the starter song it built. */
export interface StarterSong {
  genre: string
  bpm: number
  sections: { name: string; startTick: number; bars: number }[]
}

/**
 * The engine builds a starter song: its tracks, clips and tempo. The tempo
 * map is read again here, since the engine set the song's tempo itself:
 * the tempo lane and the grid kept the empty song's 140 BPM while the song
 * played at 150.
 */
export async function createStarterSong(genreId: string): Promise<StarterSong> {
  const song = await invoke<StarterSong>('create_starter_song', { genreId })
  await useTempoMapStore.getState().refresh()
  return song
}
