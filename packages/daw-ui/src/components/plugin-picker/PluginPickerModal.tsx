import { memo, useCallback, useEffect, useMemo, useRef, useState } from 'react'
import {
  usePluginCatalogStore,
  isHardwaveNative,
  type PickerCategory,
  type PluginDescriptor,
} from '../../stores/pluginCatalogStore'
import './picker.css'

export interface PluginPickerModalProps {
  /** 0-based slot index, surfaced in the header as "→ Slot N". */
  slotIndex: number
  /** Called when the user picks a row. */
  onPick: (pluginId: string) => void
  /** Called when the user presses Esc, clicks the scrim, or hits Cancel. */
  onClose: () => void
}

/**
 * 780 × 560 modal — full plug-in browser with search, category sidebar,
 * and keyboard navigation.
 *
 * Lazy-mounted from `FxRackPanel` so the bundle for the modal (and its
 * row renderer) isn't pulled in until the user clicks "Search more
 * plug-ins…". The category sidebar lists fixed buckets — All, Hardwave,
 * Instrument, Effect, Analyzer, Other — matching `PluginCategory` plus
 * the "Hardwave" vendor pseudo-category.
 *
 * Search is debounced 80 ms to keep the list paint snappy on large
 * scanner caches (5k+ plug-ins isn't unusual). Keyboard:
 *   - ↑/↓     navigate the visible rows
 *   - Enter   pick the highlighted row
 *   - Esc     close
 * Mouse hover also moves the highlight, so the two stay in sync.
 */
export const PluginPickerModal = memo(function PluginPickerModal(props: PluginPickerModalProps) {
  const { slotIndex, onPick, onClose } = props

  const plugins = usePluginCatalogStore((s) => s.plugins)
  const loading = usePluginCatalogStore((s) => s.loading)

  const [activeCat, setActiveCat] = useState<PickerCategory>('All')
  const [rawQuery, setRawQuery] = useState('')
  const [debouncedQuery, setDebouncedQuery] = useState('')
  const [highlightIdx, setHighlightIdx] = useState(0)
  const searchRef = useRef<HTMLInputElement | null>(null)
  const listRef = useRef<HTMLDivElement | null>(null)

  // 80 ms debounce — fast enough to feel live, slow enough to swallow
  // burst keystrokes.
  useEffect(() => {
    const t = window.setTimeout(() => setDebouncedQuery(rawQuery), 80)
    return () => window.clearTimeout(t)
  }, [rawQuery])

  // Focus the search input on mount; rapid-fire typing is the dominant
  // input mode for power users.
  useEffect(() => {
    searchRef.current?.focus()
  }, [])

  // Esc closes regardless of focus (mousedown on input doesn't steal
  // the keydown).
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.key === 'Escape') {
        e.preventDefault()
        onClose()
      }
    }
    document.addEventListener('keydown', onKey)
    return () => document.removeEventListener('keydown', onKey)
  }, [onClose])

  // Apply category + search filters in one pass.
  const filtered = useMemo<PluginDescriptor[]>(() => {
    const q = debouncedQuery.trim().toLowerCase()
    return plugins.filter((p) => {
      if (activeCat === 'Hardwave' && !isHardwaveNative(p)) return false
      if (activeCat !== 'All' && activeCat !== 'Hardwave' && p.category !== activeCat) return false
      if (!q) return true
      return (p.name + ' ' + p.vendor + ' ' + p.category).toLowerCase().includes(q)
    })
  }, [plugins, activeCat, debouncedQuery])

  // Reset highlight whenever the filtered list changes shape.
  useEffect(() => {
    setHighlightIdx(0)
  }, [activeCat, debouncedQuery])

  // Category counts — used in the sidebar badges.
  const counts = useMemo(() => {
    const c: Record<PickerCategory, number> = {
      All: plugins.length,
      Hardwave: 0,
      Instrument: 0,
      Effect: 0,
      Analyzer: 0,
      Other: 0,
    }
    for (const p of plugins) {
      if (isHardwaveNative(p)) c.Hardwave++
      c[p.category]++
    }
    return c
  }, [plugins])

  const pickAtHighlight = useCallback(() => {
    const target = filtered[highlightIdx]
    if (target) onPick(target.id)
  }, [filtered, highlightIdx, onPick])

  // Arrow keys on the search input drive list navigation.
  const onSearchKey = useCallback(
    (e: React.KeyboardEvent<HTMLInputElement>) => {
      if (e.key === 'ArrowDown') {
        e.preventDefault()
        setHighlightIdx((i) => Math.min(filtered.length - 1, i + 1))
      } else if (e.key === 'ArrowUp') {
        e.preventDefault()
        setHighlightIdx((i) => Math.max(0, i - 1))
      } else if (e.key === 'Enter') {
        e.preventDefault()
        pickAtHighlight()
      }
    },
    [filtered.length, pickAtHighlight],
  )

  // Keep the highlighted row in view when arrow-navigating.
  useEffect(() => {
    const el = listRef.current?.querySelector<HTMLElement>(
      `[data-row-idx="${highlightIdx}"]`,
    )
    el?.scrollIntoView({ block: 'nearest' })
  }, [highlightIdx])

  const CATEGORIES: PickerCategory[] = [
    'All', 'Hardwave', 'Instrument', 'Effect', 'Analyzer', 'Other',
  ]

  return (
    <div className="hw-pm-scrim open" onMouseDown={(e) => {
      // Click on the scrim (not the modal itself) closes.
      if (e.target === e.currentTarget) onClose()
    }}>
      <div className="hw-pm" role="dialog" aria-label="Add plug-in">
        <div className="hw-pm-head">
          <div className="hw-pm-ttl">
            Add plug-in <em>→ Slot {slotIndex + 1}</em>
          </div>
          <button className="hw-pm-x" onClick={onClose} aria-label="Close">×</button>
        </div>

        <div className="hw-pm-search">
          <span className="hw-pm-search-ico" aria-hidden="true">⌕</span>
          <input
            ref={searchRef}
            type="text"
            placeholder="Filter by name, type, vendor…"
            value={rawQuery}
            onChange={(e) => setRawQuery(e.target.value)}
            onKeyDown={onSearchKey}
          />
          <span className="hw-pm-kbd">Esc</span>
        </div>

        <div className="hw-pm-body">
          <div className="hw-pm-cats">
            {CATEGORIES.map((c) => (
              <div
                key={c}
                className={'hw-pm-cat' + (activeCat === c ? ' on' : '')}
                onClick={() => setActiveCat(c)}
              >
                <span>{c}</span>
                <span className="hw-pm-ct">{counts[c]}</span>
              </div>
            ))}
          </div>

          <div ref={listRef} className="hw-pm-list">
            {loading && filtered.length === 0 && (
              <div className="hw-pm-empty">Scanning plug-ins…</div>
            )}
            {!loading && filtered.length === 0 && (
              <div className="hw-pm-empty">No plug-ins match.</div>
            )}
            {filtered.map((p, i) => {
              const hw = isHardwaveNative(p)
              const badge = hw ? 'NATIVE' : p.format === 'Vst3' ? 'VST3' : 'CLAP'
              return (
                <div
                  key={p.id}
                  data-row-idx={i}
                  className={
                    'hw-pm-row' +
                    (hw ? ' hw' : '') +
                    (i === highlightIdx ? ' on' : '')
                  }
                  onMouseEnter={() => setHighlightIdx(i)}
                  onClick={() => onPick(p.id)}
                >
                  <span className="hw-pm-row-ic">{hw ? 'H' : '·'}</span>
                  <div>
                    <div className="hw-pm-row-nm">{p.name}</div>
                    <div className="hw-pm-row-vd">{p.vendor}</div>
                  </div>
                  <span className="hw-pm-row-vd">{p.category}</span>
                  <span className="hw-pm-row-tg">{badge}</span>
                </div>
              )
            })}
          </div>
        </div>

        <div className="hw-pm-foot">
          <span>
            Showing <em className="hw-pm-foot-count">{filtered.length}</em> plug-ins
          </span>
          <button className="hw-pm-btn" onClick={onClose}>Cancel</button>
        </div>
      </div>
    </div>
  )
})
