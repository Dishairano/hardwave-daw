import { describe, expect, it } from 'vitest'
import { useLanguageStore } from '../../i18n'
import {
  DEFAULT_SONG, finishLabel, nextStep, noteName, previousStep, stepCounter, stepStates, templateForChoice,
} from './firstRunFlow'

describe('the first run', () => {
  it('goes start, sound, controller, song when a controller is found', () => {
    expect(nextStep('start', true)).toBe('sound')
    expect(nextStep('sound', true)).toBe('controller')
    expect(nextStep('controller', true)).toBe('song')
    expect(nextStep('song', true)).toBeNull()
  })

  it('goes from sound straight to song when no controller is found', () => {
    expect(nextStep('sound', false)).toBe('song')
    expect(previousStep('song', false)).toBe('sound')
  })

  it('goes back the way it came, on every step', () => {
    expect(previousStep('song', true)).toBe('controller')
    expect(previousStep('controller', true)).toBe('sound')
    expect(previousStep('sound', true)).toBe('start')
    expect(previousStep('start', true)).toBeNull()
  })

  it('shows the steps behind as done and this one as active', () => {
    expect(stepStates('controller', true)).toEqual({
      start: 'done', sound: 'done', controller: 'active', song: 'todo',
    })
    expect(stepStates('start', true)).toEqual({
      start: 'active', sound: 'todo', controller: 'todo', song: 'todo',
    })
  })

  it('strikes Controller through when none was found', () => {
    expect(stepStates('sound', false).controller).toBe('skipped')
    expect(stepStates('song', false)).toEqual({
      start: 'done', sound: 'done', controller: 'skipped', song: 'active',
    })
  })

  it('counts four steps either way', () => {
    useLanguageStore.setState({ language: 'en' })
    expect(stepCounter('sound')).toBe('Step 2 of 4')
    expect(stepCounter('song')).toBe('Step 4 of 4')
  })

  it('picks the Hardstyle starter, so the first Play makes a sound', () => {
    expect(DEFAULT_SONG).toBe('starter:hardstyle')
  })

  it('names what the last button opens', () => {
    useLanguageStore.setState({ language: 'en' })
    expect(finishLabel('starter:hardstyle')).toBe('Open the Hardstyle starter')
    expect(finishLabel('starter:frenchcore')).toBe('Open the Frenchcore starter')
    expect(finishLabel('empty')).toBe('Open the empty project')
    expect(finishLabel('open')).toBe('Open project…')
  })

  it('opens the empty project with its drums, as the card and Skip promise', () => {
    expect(templateForChoice('empty')).toBe('drums')
    expect(templateForChoice('starter:rawstyle')).toBe('starter:rawstyle')
  })

  it('names a key the way the piano roll does', () => {
    expect(noteName(60)).toBe('C4')
    expect(noteName(48)).toBe('C3')
    expect(noteName(61)).toBe('C#4')
  })
})
