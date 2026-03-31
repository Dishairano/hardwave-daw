import { create } from 'zustand'
import { invoke } from '@tauri-apps/api/core'
import { listen } from '@tauri-apps/api/event'

interface TransportState {
  playing: boolean
  recording: boolean
  looping: boolean
  positionSamples: number
  bpm: number
  sampleRate: number

  play: () => void
  stop: () => void
  togglePlayback: () => void
  setPosition: (pos: number) => void
  setBpm: (bpm: number) => void
  startListening: () => void
}

export const useTransportStore = create<TransportState>((set, get) => ({
  playing: false,
  recording: false,
  looping: false,
  positionSamples: 0,
  bpm: 140,
  sampleRate: 48000,

  play: () => { invoke('play'); set({ playing: true }) },
  stop: () => { invoke('stop'); set({ playing: false }) },
  togglePlayback: () => {
    if (get().playing) { get().stop() } else { get().play() }
  },
  setPosition: (pos) => invoke('set_position', { position: pos }),
  setBpm: (bpm) => invoke('set_bpm', { bpm }),

  startListening: () => {
    listen<{ position: number; playing: boolean; bpm: number }>('daw:transport', (event) => {
      set({
        positionSamples: event.payload.position,
        playing: event.payload.playing,
        bpm: event.payload.bpm,
      })
    })
  },
}))
