/**
 * Report a bug without leaving the DAW.
 *
 * Posts straight to the public bug API, the same one the website form uses,
 * so a tester never has to go and find a form. Version and OS are filled in
 * from the running build, because a report without them costs a round trip to
 * ask.
 *
 * Nothing about the user's machine is attached unless they tick the box, and
 * the exact text that would be sent is shown next to it. People can only agree
 * to what they can see: a session log carries file paths, project names and
 * sample folder names, which is someone's disk laid out in our database.
 */
import { useEffect, useMemo, useState } from 'react'
import { invoke } from '@tauri-apps/api/core'
import { hw } from '../theme'
import { useNotificationStore } from '../stores/notificationStore'
import { useProjectStore } from '../stores/projectStore'

const BUG_API = 'https://hardwavestudios.com/api/bugs'
const MIN_DESCRIPTION = 10
const MAX_DESCRIPTION = 4000

interface BugReportEnv {
  version: string
  os: string
  logAvailable: boolean
}

/** The payload the API accepts. Field limits are the API's, not ours. */
interface BugPayload {
  what: string
  product: 'daw'
  version: string
  os: string
  daw: string
  expected?: string
  email?: string
  severity?: 'crash'
  context?: Record<string, unknown>
}

export function BugReportDialog({ open, onClose, afterCrash = false }: {
  open: boolean
  onClose: () => void
  /** Set when the report follows a crash: the log is then worth attaching. */
  afterCrash?: boolean
}) {
  const [env, setEnv] = useState<BugReportEnv | null>(null)
  const [what, setWhat] = useState('')
  const [expected, setExpected] = useState('')
  const [email, setEmail] = useState('')
  const [attachLog, setAttachLog] = useState(afterCrash)
  const [logTail, setLogTail] = useState('')
  const [showLog, setShowLog] = useState(false)
  const [sending, setSending] = useState(false)
  const projectPath = useProjectStore(s => s.filePath)

  // Just the file name: a full path says where someone keeps their work, and
  // the name alone is what makes a report reproducible.
  const projectName = useMemo(
    () => (projectPath ? projectPath.split(/[\\/]/).pop() ?? null : null),
    [projectPath],
  )

  useEffect(() => {
    if (!open) return
    setAttachLog(afterCrash)
    invoke<BugReportEnv>('bug_report_env').then(setEnv).catch(() => setEnv(null))
  }, [open, afterCrash])

  useEffect(() => {
    if (!open || !attachLog) return
    invoke<string>('session_log_tail').then(setLogTail).catch(() => setLogTail(''))
  }, [open, attachLog])

  const payload = useMemo<BugPayload | null>(() => {
    if (!env) return null
    const body: BugPayload = {
      what: what.trim(),
      product: 'daw',
      version: env.version,
      os: env.os,
      daw: 'Hardwave DAW',
    }
    if (expected.trim()) body.expected = expected.trim().slice(0, 600)
    if (email.trim()) body.email = email.trim().slice(0, 255)
    if (afterCrash) body.severity = 'crash'
    if (attachLog) {
      body.context = {
        ...(projectName ? { project: projectName } : {}),
        ...(logTail ? { log: logTail } : {}),
      }
    }
    return body
  }, [env, what, expected, email, attachLog, logTail, projectName, afterCrash])

  if (!open) return null

  const tooShort = what.trim().length < MIN_DESCRIPTION
  const push = useNotificationStore.getState().push

  const send = async () => {
    if (!payload || tooShort || sending) return
    setSending(true)
    try {
      const resp = await fetch(BUG_API, {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify(payload),
      })
      if (resp.status === 429) {
        push('warning', 'Too many reports for now', {
          detail: 'The bug form accepts six reports an hour. Try again a little later, or use hardwavestudios.com/report-a-bug.',
        })
        return
      }
      if (!resp.ok) {
        // Say what came back rather than a generic failure: a rejected report
        // usually means a field the user can fix.
        const detail = await resp.text().catch(() => '')
        push('error', 'Could not send the report', {
          detail: detail.slice(0, 300) || `The server answered ${resp.status}.`,
          sticky: true,
        })
        return
      }
      const body = await resp.json() as { ok?: boolean; id?: number | string }
      push('info', body?.id ? `Reported as #${body.id}` : 'Report sent', {
        detail: 'Thank you. If you left an email we may come back to you about it.',
      })
      setWhat('')
      setExpected('')
      onClose()
    } catch (e) {
      push('error', 'Could not reach the bug report service', {
        detail: `${String(e)}\nYou can report it at hardwavestudios.com/report-a-bug instead.`,
        sticky: true,
      })
    } finally {
      setSending(false)
    }
  }

  const field: React.CSSProperties = {
    width: '100%', padding: '6px 8px', fontSize: 11,
    background: 'rgba(255,255,255,0.04)', color: hw.textPrimary,
    border: `1px solid ${hw.border}`, borderRadius: hw.radius.md, outline: 'none',
    fontFamily: 'inherit', resize: 'vertical',
  }
  const label: React.CSSProperties = {
    fontSize: 9, color: hw.textFaint, textTransform: 'uppercase',
    letterSpacing: 0.5, marginBottom: 3, display: 'block',
  }

  return (
    <div
      onMouseDown={onClose}
      style={{
        position: 'fixed', inset: 0, zIndex: 11000,
        background: 'rgba(0,0,0,0.5)',
        display: 'flex', alignItems: 'center', justifyContent: 'center',
      }}
    >
      <div
        onMouseDown={e => e.stopPropagation()}
        data-testid="bug-report-dialog"
        style={{
          width: 520, maxHeight: '86vh', overflowY: 'auto', padding: 16,
          background: 'rgba(14,14,20,0.98)',
          border: `1px solid ${hw.borderLight}`,
          borderRadius: hw.radius.lg,
          boxShadow: '0 20px 60px rgba(0,0,0,0.6)',
        }}
      >
        <div style={{ fontSize: 13, color: hw.textPrimary, fontWeight: 600, marginBottom: 2 }}>
          Report a bug
        </div>
        <div style={{ fontSize: 10, color: hw.textFaint, marginBottom: 12 }}>
          {env
            ? `Hardwave DAW ${env.version} on ${env.os}. Sent to the Hardwave bug tracker.`
            : 'Sent to the Hardwave bug tracker.'}
        </div>

        <label style={label}>What happened</label>
        <textarea
          autoFocus
          rows={5}
          value={what}
          maxLength={MAX_DESCRIPTION}
          onChange={e => setWhat(e.target.value)}
          placeholder="What you did, and what the DAW did instead."
          style={field}
        />
        <div style={{ fontSize: 9, color: tooShort && what.length > 0 ? hw.yellow : hw.textFaint, margin: '3px 0 10px' }}>
          {tooShort && what.length > 0
            ? `A few more words, please (at least ${MIN_DESCRIPTION} characters).`
            : `${what.length} / ${MAX_DESCRIPTION}`}
        </div>

        <label style={label}>What you expected (optional)</label>
        <textarea rows={2} value={expected} maxLength={600} onChange={e => setExpected(e.target.value)} style={{ ...field, marginBottom: 10 }} />

        <label style={label}>Your email (optional, so we can come back to you)</label>
        <input value={email} maxLength={255} onChange={e => setEmail(e.target.value)} style={{ ...field, marginBottom: 12 }} />

        <div style={{
          padding: 10, marginBottom: 12,
          background: 'rgba(255,255,255,0.03)',
          border: `1px solid ${hw.border}`, borderRadius: hw.radius.md,
        }}>
          <label style={{ display: 'flex', gap: 8, alignItems: 'flex-start', cursor: 'pointer' }}>
            <input
              type="checkbox"
              checked={attachLog}
              disabled={env ? !env.logAvailable : true}
              onChange={e => setAttachLog(e.target.checked)}
              style={{ marginTop: 2 }}
            />
            <span style={{ fontSize: 10, color: hw.textSecondary }}>
              Attach the end of this session's log
              {projectName ? ' and the project name' : ''}
              {afterCrash && (
                <span style={{ color: hw.yellow }}>
                  {' '}— ticked because this follows a crash, where the log usually holds the cause
                </span>
              )}
              <div style={{ color: hw.textFaint, marginTop: 3 }}>
                {env && !env.logAvailable
                  ? 'No session log was created for this run.'
                  : 'It can contain file paths and sample folder names from your computer.'}
              </div>
            </span>
          </label>
          {attachLog && (
            <div style={{ marginTop: 8 }}>
              <button
                onClick={() => setShowLog(v => !v)}
                style={{
                  fontSize: 9, color: hw.accent, background: 'transparent',
                  border: 'none', cursor: 'pointer', padding: 0,
                }}
              >
                {showLog ? 'Hide what will be sent' : 'Show exactly what will be sent'}
              </button>
              {showLog && (
                <pre style={{
                  marginTop: 6, maxHeight: 200, overflow: 'auto',
                  fontSize: 9, lineHeight: 1.4, color: hw.textFaint,
                  fontFamily: hw.font.mono, whiteSpace: 'pre-wrap', wordBreak: 'break-all',
                }}>
                  {JSON.stringify(payload?.context ?? {}, null, 2)}
                </pre>
              )}
            </div>
          )}
        </div>

        <div style={{ display: 'flex', gap: 8, justifyContent: 'flex-end', alignItems: 'center' }}>
          <span style={{ flex: 1, fontSize: 9, color: hw.textFaint }}>
            Also at hardwavestudios.com/report-a-bug
          </span>
          <button
            onClick={onClose}
            style={{
              padding: '5px 12px', fontSize: 10, color: hw.textSecondary,
              background: 'transparent', border: `1px solid ${hw.border}`,
              borderRadius: hw.radius.md, cursor: 'pointer',
            }}
          >
            Cancel
          </button>
          <button
            onClick={send}
            disabled={tooShort || sending}
            data-testid="bug-report-send"
            style={{
              padding: '5px 14px', fontSize: 10, fontWeight: 600,
              color: tooShort || sending ? hw.textFaint : '#0b0b10',
              background: tooShort || sending ? 'rgba(255,255,255,0.06)' : hw.accent,
              border: 'none', borderRadius: hw.radius.md,
              cursor: tooShort || sending ? 'default' : 'pointer',
            }}
          >
            {sending ? 'Sending…' : 'Send report'}
          </button>
        </div>
      </div>
    </div>
  )
}
