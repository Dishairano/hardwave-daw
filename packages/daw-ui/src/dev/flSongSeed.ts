/**
 * Dev-only: the founder's FL song "RyZe Take U TIme" in FL's shape, the way
 * mockups M2 (mixer) and M3 (rack) draw it, for the screenshot harness
 * (?song=fl). The data is the mockups' own: the master and inserts 1 to 20
 * with FL's names, colours and effects (7 with OTT, 9 to 16 named with
 * nothing in them), and FL's 106 channels, each routed to its insert or
 * straight into the master. Nothing here ships: screenshot.tsx is the only
 * importer.
 */
import type { TrackWithClips, InsertInfo } from '../stores/trackStore'

/** [index, name, colour, group, kind, insert it plays into (0 the master)] */
type RackRow = [number, string, string, number, 'Sampler' | 'Instrument' | 'AudioClip' | 'Automation', number]
const RACK: RackRow[] = [
  [0, "808 Kick", "#5a486d", 2, "Sampler", 1],
  [1, "808 Clap", "#5a486d", 2, "Sampler", 2],
  [2, "808 HiHat", "#5a486d", 2, "Sampler", 3],
  [3, "808 Snare", "#5a486d", 2, "Sampler", 4],
  [4, "FLEX Bass", "#485156", 2, "Instrument", 5],
  [5, "KIT_01_Kick_02_126BPM_A#MAJ", "#7c141f", 0, "AudioClip", 4],
  [6, "KIT_01_Percussion_126BPM_A#MAJ", "#ac2660", 0, "AudioClip", 0],
  [7, "KIT_02_Bass_02_128BPM_A#MAJ", "#90142b", 0, "AudioClip", 0],
  [8, "Djs From Mars BB3 - HiHat09", "#ac2660", 0, "AudioClip", 0],
  [9, "KIT_02_Bass_Group_128BPM_A#MAJ", "#ac3a68", 0, "AudioClip", 0],
  [10, "KIT_02_Clap_128BPM_A#MAJ", "#7c141f", 0, "AudioClip", 0],
  [11, "91V_PUKG_132_vocal_phrase_chop_take_time_wet_Gm", "#7a143f", 0, "AudioClip", 0],
  [12, "AU_HC_fx_downlifter_one_shot_plethoric", "#af546d", 0, "AudioClip", 0],
  [13, "AU_HC_fx_uplifter_one_shot_reversive", "#ac2660", 0, "AudioClip", 0],
  [14, "AU_HC2_fx_uplifter_abstruse_G#", "#ac3a68", 0, "AudioClip", 0],
  [15, "FF_ET_reverse_slow_down_F#", "#b11434", 0, "AudioClip", 0],
  [16, "KSHMR_sok5_stomp_gritty", "#7a143f", 0, "AudioClip", 0],
  [17, "IT_Drum_Loop_15_Kick_Loop_135", "#b11434", 0, "AudioClip", 0],
  [18, "IT_Drum_Loop_15_Kick_Loop_135 #2", "#b11434", 0, "AudioClip", 0],
  [19, "HTR2 Sub Bass Loop 145BPM 8", "#a4143f", 0, "AudioClip", 0],
  [20, "Acid 145 BPM 13 - E Phrygian", "#2d1414", 0, "AudioClip", 0],
  [21, "Running Bassline Top  Melody 145 BPM 12 - F# Locrian", "#64141b", 0, "AudioClip", 0],
  [22, "hdcr_synth_loop_vintage_01_g_122", "#2d1414", 0, "AudioClip", 0],
  [23, "126_RvbBeat_01_663", "#2e1414", 0, "AudioClip", 0],
  [24, "SS_FFTH_clap_round_one_shot_night", "#a4143f", 0, "AudioClip", 0],
  [25, "RU_SR_140_plucked_dry_town_Dmin", "#2d1414", 0, "AudioClip", 0],
  [26, "ZEN_PDT_134_lead_synth_loop_acido_D", "#90142b", 0, "AudioClip", 0],
  [27, "ph2_drm128_minimax_ful", "#ae145e", 0, "AudioClip", 0],
  [28, "91V_SF_128_vocal_hook_pitched_lost_retro_wet_Am", "#7a143f", 0, "AudioClip", 0],
  [29, "91V_VHA_128_vocal_hook_knock_me_off_my_feel_wet_Bm", "#90142b", 0, "AudioClip", 0],
  [30, "DS_BLINDERS_128_vocal_harmony_female_taste_chorus_wet_F#min", "#4c1418", 0, "AudioClip", 0],
  [31, "smth_vox128_melodix", "#90142b", 0, "AudioClip", 0],
  [32, "THM - Attitude - Push That Pedal - Layered FX - D#min - 126 BPM - WET", "#8b3b62", 0, "AudioClip", 19],
  [33, "THM - Attitude - Push That Pedal - Layered FX - D#min - 126 BPM - WET #7", "#8b3b62", 0, "AudioClip", 0],
  [34, "THM - Attitude - Push That Pedal - Layered FX - D#min - 126 BPM - WET #6", "#8b3b62", 0, "AudioClip", 0],
  [35, "THM - Attitude - Push That Pedal - Layered FX - D#min - 126 BPM - WET #5", "#8b3b62", 0, "AudioClip", 0],
  [36, "THM - Attitude - Push That Pedal - Layered FX - D#min - 126 BPM - WET #3", "#8b3b62", 0, "AudioClip", 0],
  [37, "THM - Attitude - Push That Pedal - Layered FX - D#min - 126 BPM - WET #4", "#8b3b62", 0, "AudioClip", 0],
  [38, "THM - Attitude - Push That Pedal - Layered FX - D#min - 126 BPM - WET #2", "#8b3b62", 0, "AudioClip", 0],
  [39, "THM - Downlifter 14 - 124 BPM", "#b11434", 0, "AudioClip", 0],
  [40, "THM - White Noise Uplifter 3 - 125 BPM - 4 Bars", "#4c1418", 0, "AudioClip", 0],
  [41, "TOP_LOOP_135BPM (2)", "#4c1418", 0, "AudioClip", 0],
  [42, "Fourfloor_FFSP001_Kick_Loop_129BPM_RAW_4", "#2e1414", 0, "AudioClip", 0],
  [43, "Percussion 10", "#90142b", 0, "AudioClip", 8],
  [44, "Djs From Mars BB3 - HiHat21", "#90142b", 0, "AudioClip", 0],
  [45, "THM - Attitude - Instrumental 29 - C#min - 126 BPM", "#ac3a68", 0, "AudioClip", 0],
  [46, "THM - Crash 1", "#ac3a68", 0, "AudioClip", 1],
  [47, "THM - Top Loop 45 - 128 BPM", "#a4143f", 0, "AudioClip", 0],
  [48, "THM - Pad 11 - A# - 127 BPM", "#ac3a68", 0, "AudioClip", 1],
  [49, "Techhousemarket - Impact 4", "#ac3a68", 0, "AudioClip", 0],
  [50, "THM - Attitude - Percussion Loop 7 - 126BPM", "#64141b", 0, "AudioClip", 0],
  [51, "THM - Top Loop 45 - 128 BPM #2", "#a4143f", 0, "AudioClip", 0],
  [52, "11 BASS LOOP 126BPM D#min", "#ac2660", 0, "AudioClip", 6],
  [53, "Victek_TP_Buildups_5_SidechainNoise", "#2d1414", 0, "AudioClip", 0],
  [54, "Victek_TP_SynthLoops_4_Rave_F#minor", "#a4143f", 0, "AudioClip", 0],
  [55, "TKNVLT_HTSP1_HIHATLOOPS_150_3", "#aa144a", 0, "AudioClip", 0],
  [56, "THM_LTN - Hat Loop - Soft Attack - 129 BPM", "#a4143f", 0, "AudioClip", 4],
  [57, "TKNVLT_HTVIP1_HIHATLOOPS_150_2", "#90142b", 0, "AudioClip", 0],
  [58, "TKNVLT_HTVIP1_HIHATLOOPS_150_4", "#90142b", 0, "AudioClip", 0],
  [59, "VS_OGKush_Kick_BuilderLoops_1_150BPM", "#2d1414", 0, "AudioClip", 5],
  [60, "CB_Clap", "#2d1414", 0, "AudioClip", 0],
  [61, "TKNVLT_HVE1_VOCAL_LOOP_47_145", "#7a143f", 0, "AudioClip", 0],
  [62, "TKNVLT_HVE1_VOCAL_LOOP_2_145", "#6b1417", 0, "AudioClip", 0],
  [63, "THM - Percussion Loop - Layer With Kick - 129 BPM", "#af546d", 0, "AudioClip", 0],
  [64, "VS_MelodicHardTechno_Kicks_Loops_2_158BPM", "#8b3b62", 0, "AudioClip", 0],
  [65, "VS_MelodicHardTechno_Percussion_Loops_3_158BPM", "#90142b", 0, "AudioClip", 0],
  [66, "VS_MelodicHardTechno_Ride_Loops_1_155BPM", "#aa144a", 0, "AudioClip", 0],
  [67, "TKNVLT_HSA1_SYNTH_LOOP_14_145_C", "#2e1414", 0, "AudioClip", 0],
  [68, "TKNVLT_HGE1_MELODIC_LOOP_2_E_145", "#8b3b62", 0, "AudioClip", 1],
  [69, "VS_OGKush_Bassline_Loops_5_D#_150BPM", "#a4143f", 0, "AudioClip", 0],
  [70, "JKid_Impact_12", "#b11434", 0, "AudioClip", 0],
  [71, "LLTXS_fx_one_shot_vinyl_stop_02", "#ac3a68", 0, "AudioClip", 0],
  [72, "NW_OSHRS_80_kit_loop_krakenfall_brass_Dmin", "#ac3a68", 0, "AudioClip", 2],
  [73, "PM_EN_FX_Reverse_One_Shot_Casing", "#7c141f", 0, "AudioClip", 0],
  [74, "WhooshGhostlyHowl_S011HO._2", "#2d1414", 0, "AudioClip", 0],
  [75, "FXT_fx_alien", "#2e1414", 0, "AudioClip", 0],
  [76, "FXT_fx_alien - Kanal-Panning", "#7e1443", 1, "Automation", 0],
  [77, "Fruity Parametric EQ 2 (Insert 3) - Stumm / solo", "#ae144e", 1, "Automation", 0],
  [78, "ESM_Cinematic_Midrange_Airy_Swoosh_3_Whoosh_Reveal_Accent_Movie_", "#aa144a", 0, "AudioClip", 0],
  [79, "AW2_War_Drums_140bpm_5", "#b11434", 0, "AudioClip", 0],
  [80, "PM_PS_Fx_One_Shot_Uplifter_Big_F", "#2d1414", 0, "AudioClip", 3],
  [81, "PM_PS_150_Drum_Loop_Fill_Breakbeat", "#a4143f", 0, "AudioClip", 0],
  [82, "DS_MT2_125_vocal_harmonies_male_loop_city_bridge_double_wet_Bbmin", "#a4143f", 0, "AudioClip", 0],
  [83, "VS_OGKush_Bassline_Loops_3_G_150BPM", "#2d1414", 0, "AudioClip", 0],
  [84, "04_HS_Ride", "#90142b", 0, "AudioClip", 0],
  [85, "KIT_03_Kick_With_Bass_125BPM_FMAJ", "#ac3a68", 0, "AudioClip", 0],
  [86, "KIT_03_Ride_125BPM_FMAJ", "#a4143f", 0, "AudioClip", 0],
  [87, "TKNVLT_HGE1_BASS_LOOP_7_E_145", "#a4143f", 0, "AudioClip", 0],
  [88, "VS_SoundDesignSessions1_FX_Loops_1_155BPM", "#6b1417", 0, "AudioClip", 7],
  [89, "Victek_TP_FX_20", "#ac3a68", 0, "AudioClip", 0],
  [90, "Victek_TP_PercLoops_130BPM_17_NoisyPerc", "#aa144a", 0, "AudioClip", 0],
  [91, "TKNVLT_HGE1_FX_MISC_19", "#b11434", 0, "AudioClip", 4],
  [92, "TKNVLT_HGE1_NOISE_STUTTER_14_E_145", "#ac2660", 0, "AudioClip", 0],
  [93, "TKNVLT_HGE1_MELODIC_LOOP_9_E_145", "#90142b", 0, "AudioClip", 0],
  [94, "TKNVLT_HGE1_ARP - Hollow", "#2d1414", 0, "AudioClip", 0],
  [95, "IT_Drum_Loop_15_Kick_Loop_135 #2 - Channel volume", "#8e6060", 1, "Automation", 0],
  [96, "IT_Drum_Loop_15_Kick_Loop_135 #2 - Channel pitch", "#8e6088", 1, "Automation", 0],
  [97, "789AFDK_CRASH_6", "#68797e", 0, "AudioClip", 17],
  [98, "909 Snare", "#686d7e", 0, "AudioClip", 18],
  [99, "909 Snare - Channel pitch", "#8e6088", 1, "Automation", 0],
  [100, "909 Snare - Channel volume", "#8e6060", 1, "Automation", 0],
  [101, "OPS_FHFX2_SUB_DROP_01_150_D#", "#707e68", 0, "AudioClip", 0],
  [102, "909 Clap", "#7e7068", 0, "AudioClip", 0],
  [103, "Endless Smile - Master - Intensit", "#6d608e", 1, "Automation", 0],
  [104, "Master - Volume", "#608e8b", 1, "Automation", 0],
  [105, "Fruity Reeverb 2 (Insert 3) - Mix level", "#60698e", 1, "Automation", 0],
]

/** The master (0) and FL's inserts 1 to 20: name ('' for none), colour, effects. */
const MIX: { n: number; name: string; color: string | null; fx: string[] }[] = [
  {"n": 0, "name": "Master", "color": "#e5243b", "fx": ["Endless Smile"]},
  {"n": 1, "name": "", "color": null, "fx": ["Fruity Reeverb 2"]},
  {"n": 2, "name": "", "color": null, "fx": ["Fruity Reeverb 2"]},
  {"n": 3, "name": "", "color": null, "fx": ["Fruity Reeverb 2"]},
  {"n": 4, "name": "", "color": null, "fx": ["Fruity Parametric EQ 2"]},
  {"n": 5, "name": "", "color": null, "fx": ["Fruity Parametric EQ 2"]},
  {"n": 6, "name": "THM - Crash 1", "color": "#ac3a68", "fx": ["Fresh Air"]},
  {"n": 7, "name": "THM - Pad 11 - A# - 127 BPM", "color": "#ac3a68", "fx": ["OTT"]},
  {"n": 8, "name": "Percussion 10", "color": "#90142b", "fx": []},
  {"n": 9, "name": "TKNVLT_HGE1_MELODIC_LOOP_2_E_145", "color": "#8b3b62", "fx": []},
  {"n": 10, "name": "TKNVLT_HGE1_MELODIC_LOOP_2_E_145", "color": "#8b3b62", "fx": []},
  {"n": 11, "name": "NW_OSHRS_80_kit_loop_krakenfall_brass_Dmin", "color": "#ac3a68", "fx": []},
  {"n": 12, "name": "NW_OSHRS_80_kit_loop_krakenfall_brass_Dmin", "color": "#ac3a68", "fx": []},
  {"n": 13, "name": "NW_OSHRS_80_kit_loop_krakenfall_brass_Dmin", "color": "#ac3a68", "fx": []},
  {"n": 14, "name": "KIT_01_Kick_02_126BPM_A#MAJ", "color": "#7c141f", "fx": []},
  {"n": 15, "name": "VS_OGKush_Kick_BuilderLoops_1_150BPM", "color": "#2d1414", "fx": []},
  {"n": 16, "name": "VS_SoundDesignSessions1_FX_Loops_1_155BPM", "color": "#6b1417", "fx": []},
  {"n": 17, "name": "789AFDK_CRASH_6", "color": "#68797e", "fx": []},
  {"n": 18, "name": "909 Snare", "color": "#686d7e", "fx": []},
  {"n": 19, "name": "THM - Attitude - Push That Pedal - Layered FX - D#min - 126 BPM - WET", "color": "#8b3b62", "fx": ["Pro-Q 4"]},
  {"n": 20, "name": "KICK SIDECHAIN REVERB", "color": "#763614", "fx": ["Pro-Q 3", "ValhallaRoom", "Pro-Q 3", "Fruity Limiter"]},
]

/** FL's rack groups, as the song names them. */
export const FL_RACK_GROUPS = ['Audio', 'Automation', 'Unsorted']

function slot(owner: string, name: string, i: number): InsertInfo {
  return { id: `${owner}-fx-${i}`, pluginId: `fl.${name.toLowerCase().replace(/[^a-z0-9]+/g, '-')}`, pluginName: name, enabled: true, wet: 1, sidechainSource: null }
}

function base(id: string, name: string, kind: string, color: string): TrackWithClips {
  return {
    id, name, kind, color,
    volume_db: 0, pan: 0, muted: false, soloed: false, armed: false, solo_safe: false,
    monitorInput: false, phaseInvert: false, swapLr: false, stereoSeparation: 1,
    delaySamples: 0, pitchSemitones: 0, fineTuneCents: 0,
    filterType: 'none', filterCutoffHz: 20000, filterResonance: 0,
    outputBus: null, insert_count: 0, inserts: [], automationLanes: [], automationClips: [],
    clips: [], mixerNumber: null, rackGroup: null,
  } as unknown as TrackWithClips
}

/** The song's tracks: the master, the 20 inserts, then the 106 channels in
 *  FL's order. */
export function flSongTracks(): TrackWithClips[] {
  const tracks: TrackWithClips[] = []
  for (const m of MIX) {
    const id = m.n === 0 ? 'master' : `fl-ins-${m.n}`
    const t = base(id, m.n === 0 ? 'Master' : m.name, m.n === 0 ? 'Master' : 'Bus', m.color ?? (m.n === 0 ? '#e5243b' : '#52525b'))
    t.mixerNumber = m.n
    t.inserts = m.fx.map((f, i) => slot(id, f, i))
    t.insert_count = t.inserts.length
    tracks.push(t)
  }
  for (const [i, name, color, group, kind, insert] of RACK) {
    const t = base(`fl-ch-${i}`, name, kind === 'AudioClip' ? 'Audio' : kind === 'Automation' ? 'Automation' : 'Midi', color)
    t.rackGroup = group
    t.outputBus = insert > 0 ? `fl-ins-${insert}` : null
    t.muted = kind === 'Instrument'
    tracks.push(t)
  }
  return tracks
}
