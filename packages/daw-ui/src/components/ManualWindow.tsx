import { useMemo, useState } from 'react'
import manualSource from '../../../../docs/MANUAL.md?raw'
import { hw } from '../theme'
import { DialogFrame } from './ui/DialogFrame'

/**
 * The manual, in the app.
 *
 * There were help overlays and a README, and a Help menu entry pointing
 * at a wiki that has nothing in it. Neither is something a new user can
 * read from start to finish, so the manual ships with the program and
 * is the same file the repository carries.
 */

interface Section {
  title: string
  lines: string[]
}

/** Split the manual on its "## " headings, keeping the order. */
function sections(source: string): Section[] {
  const out: Section[] = []
  let current: Section | null = null
  for (const line of source.split('\n')) {
    if (line.startsWith('## ')) {
      current = { title: line.slice(3).trim(), lines: [] }
      out.push(current)
      continue
    }
    if (line.startsWith('# ')) continue
    if (current) current.lines.push(line)
  }
  return out
}

/** Bold and inline code, which is all the manual uses inside a line. */
function inline(text: string, key: number) {
  const parts = text.split(/(\*\*[^*]+\*\*|`[^`]+`)/g)
  return (
    <span key={key}>
      {parts.map((part, i) => {
        if (part.startsWith('**') && part.endsWith('**')) {
          return <strong key={i} style={{ color: hw.textPrimary }}>{part.slice(2, -2)}</strong>
        }
        if (part.startsWith('`') && part.endsWith('`')) {
          return (
            <code key={i} style={{
              fontFamily: hw.font.mono, fontSize: 12.5,
              background: 'rgba(255,255,255,0.06)', padding: '1px 4px', borderRadius: 3,
            }}>{part.slice(1, -1)}</code>
          )
        }
        return <span key={i}>{part}</span>
      })}
    </span>
  )
}

function Body({ lines }: { lines: string[] }) {
  const blocks: React.ReactNode[] = []
  let paragraph: string[] = []
  let list: string[] = []
  let ordered = false

  const flushParagraph = () => {
    if (paragraph.length === 0) return
    blocks.push(
      <p key={`p${blocks.length}`} style={{ margin: '0 0 10px', lineHeight: 1.6 }}>
        {inline(paragraph.join(' '), 0)}
      </p>,
    )
    paragraph = []
  }
  const flushList = () => {
    if (list.length === 0) return
    const items = list.map((item, i) => (
      <li key={i} style={{ marginBottom: 4, lineHeight: 1.55 }}>{inline(item, i)}</li>
    ))
    blocks.push(
      ordered
        ? <ol key={`l${blocks.length}`} style={{ margin: '0 0 10px 18px', padding: 0 }}>{items}</ol>
        : <ul key={`l${blocks.length}`} style={{ margin: '0 0 10px 18px', padding: 0 }}>{items}</ul>,
    )
    list = []
  }

  for (const raw of lines) {
    const line = raw.trimEnd()
    if (line.trim() === '') { flushParagraph(); flushList(); continue }
    const bullet = line.match(/^- (.*)$/)
    const numbered = line.match(/^\d+\. (.*)$/)
    if (bullet) {
      flushParagraph()
      if (ordered && list.length > 0) flushList()
      ordered = false
      list.push(bullet[1])
      continue
    }
    if (numbered) {
      flushParagraph()
      if (!ordered && list.length > 0) flushList()
      ordered = true
      list.push(numbered[1])
      continue
    }
    // A continuation line of a bullet is indented in the source.
    if (line.startsWith('  ') && list.length > 0) {
      list[list.length - 1] += ` ${line.trim()}`
      continue
    }
    flushList()
    paragraph.push(line.trim())
  }
  flushParagraph()
  flushList()
  return <>{blocks}</>
}

export function ManualWindow({ onClose }: { onClose: () => void }) {
  const all = useMemo(() => sections(manualSource), [])
  const [query, setQuery] = useState('')
  const [active, setActive] = useState(0)

  const needle = query.trim().toLowerCase()
  const matches = useMemo(() => {
    if (needle === '') return all.map((_, i) => i)
    return all
      .map((s, i) => ({ s, i }))
      .filter(({ s }) =>
        s.title.toLowerCase().includes(needle)
        || s.lines.join(' ').toLowerCase().includes(needle))
      .map(({ i }) => i)
  }, [all, needle])

  const shown = matches.includes(active) ? active : (matches[0] ?? 0)
  const section = all[shown]

  return (
    <DialogFrame
      title="Manual"
      subtitle="What the program does today"
      onClose={onClose}
      width={860}
      height="80vh"
      headerActions={
        <input
          className="hw-input"
          value={query}
          onChange={(e) => setQuery(e.target.value)}
          placeholder="Search the manual"
          style={{ width: 200 }}
        />
      }
    >
      <div style={{ display: 'flex', flex: 1, minHeight: 0 }}>
        <div style={{
          width: 210, flexShrink: 0, overflowY: 'auto',
          borderRight: `1px solid ${hw.border}`, background: hw.bgElevated, padding: 6,
        }}>
          {matches.length === 0 && (
            <div style={{ fontSize: 12, color: hw.textFaint, padding: 8 }}>
              Nothing in the manual mentions that.
            </div>
          )}
          {matches.map(i => (
            <div
              key={i}
              onClick={() => setActive(i)}
              style={{
                padding: '6px 8px', fontSize: 12.5, cursor: 'pointer',
                borderRadius: hw.radius.sm,
                color: i === shown ? hw.textPrimary : hw.textMuted,
                background: i === shown ? 'rgba(255,255,255,0.06)' : 'transparent',
                borderLeft: `2px solid ${i === shown ? hw.accent : 'transparent'}`,
              }}
            >
              {all[i].title}
            </div>
          ))}
        </div>

        <div style={{ flex: 1, minWidth: 0, overflowY: 'auto', padding: '16px 20px', fontSize: 12 }}>
          {section && (
            <>
              <h2 style={{ fontSize: 15, margin: '0 0 12px' }}>{section.title}</h2>
              <Body lines={section.lines} />
            </>
          )}
        </div>
      </div>
    </DialogFrame>
  )
}

