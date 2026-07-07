import { useState } from 'react'

/**
 * All of App.tsx's dialog/panel visibility flags in one place.
 *
 * Extracted from a 20-line useState pile in the App body (deep-research
 * P2-13) — the returned names are identical to the old locals so every
 * existing call site destructures unchanged. Panels whose visibility
 * must survive reloads (touch controller, mixer variant) stay in their
 * own persisted stores; these are session-only.
 */
export function useAppDialogs() {
  const [showBrowser, setShowBrowser] = useState(true)
  const [showMixer, setShowMixer] = useState(false)
  const [showChannelRack, setShowChannelRack] = useState(false)
  const [showPlaylist, setShowPlaylist] = useState(true)
  const [showPianoRoll, setShowPianoRoll] = useState(false)
  const [showRoadmap, setShowRoadmap] = useState(false)
  const [showAudioSettings, setShowAudioSettings] = useState(false)
  const [showThemePicker, setShowThemePicker] = useState(false)
  const [showAbout, setShowAbout] = useState(false)
  const [showShortcuts, setShowShortcuts] = useState(false)
  const [showHelp, setShowHelp] = useState(false)
  const [showTrackTemplateManager, setShowTrackTemplateManager] = useState(false)
  const [showLoudness, setShowLoudness] = useState(false)
  const [showOscilloscope, setShowOscilloscope] = useState(false)
  const [showSpectrum, setShowSpectrum] = useState(false)
  const [showProjectInfo, setShowProjectInfo] = useState(false)
  const [showTempoTapper, setShowTempoTapper] = useState(false)
  const [showMidiMappings, setShowMidiMappings] = useState(false)
  const [showTempoMap, setShowTempoMap] = useState(false)
  const [showHistory, setShowHistory] = useState(false)
  const [showDevPanel, setShowDevPanel] = useState(false)

  return {
    showBrowser, setShowBrowser,
    showMixer, setShowMixer,
    showChannelRack, setShowChannelRack,
    showPlaylist, setShowPlaylist,
    showPianoRoll, setShowPianoRoll,
    showRoadmap, setShowRoadmap,
    showAudioSettings, setShowAudioSettings,
    showThemePicker, setShowThemePicker,
    showAbout, setShowAbout,
    showShortcuts, setShowShortcuts,
    showHelp, setShowHelp,
    showTrackTemplateManager, setShowTrackTemplateManager,
    showLoudness, setShowLoudness,
    showOscilloscope, setShowOscilloscope,
    showSpectrum, setShowSpectrum,
    showProjectInfo, setShowProjectInfo,
    showTempoTapper, setShowTempoTapper,
    showMidiMappings, setShowMidiMappings,
    showTempoMap, setShowTempoMap,
    showHistory, setShowHistory,
    showDevPanel, setShowDevPanel,
  }
}
