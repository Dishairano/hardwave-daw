import { create } from 'zustand'
import { persist } from 'zustand/middleware'

import { nl } from './nl'

/**
 * Translation, without a library and without a build step.
 *
 * Everything in the app is written in English in the source. A
 * translation is a map from that English text to the other language,
 * so a string that nobody has translated yet still reads correctly
 * rather than showing a key like `menu.file.save`. Adding a language is
 * adding a file; adding a string needs nothing at all.
 */

export type LanguageId = 'en' | 'nl'

export const LANGUAGES: { id: LanguageId; label: string }[] = [
  { id: 'en', label: 'English' },
  { id: 'nl', label: 'Nederlands' },
]

const DICTIONARIES: Record<LanguageId, Record<string, string>> = {
  en: {},
  nl,
}

interface LanguageState {
  language: LanguageId
  setLanguage: (id: LanguageId) => void
}

export const useLanguageStore = create<LanguageState>()(
  persist(
    (set) => ({
      language: 'en',
      setLanguage: (language) => set({ language }),
    }),
    { name: 'hw-language' },
  ),
)

/**
 * The text to show for a piece of English.
 *
 * Called outside React as well as inside it, so it reads the store
 * directly rather than being a hook. A component that has to re-render
 * when the language changes subscribes with `useLanguage()`.
 */
export function t(english: string): string {
  const { language } = useLanguageStore.getState()
  if (language === 'en') return english
  return DICTIONARIES[language]?.[english] ?? english
}

/** Subscribe to the chosen language, for components that show text. */
export function useLanguage(): LanguageId {
  return useLanguageStore(s => s.language)
}

/**
 * How much of a language is done, for the settings page to say so
 * rather than claiming a half-finished translation is finished.
 */
export function coverage(id: LanguageId): number {
  return Object.keys(DICTIONARIES[id] ?? {}).length
}
