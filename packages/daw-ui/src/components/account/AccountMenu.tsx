import { useEffect, useLayoutEffect, useRef, useState } from 'react'
import { createPortal } from 'react-dom'
import { t, tf, useLanguage } from '../../i18n'
import {
  useAccountStore, watchAccount, initialsOf,
  type Account, type AccountStatus, type Plan,
} from '../../stores/accountStore'
import './account.css'

/**
 * The Hardwave account at the top right of the DAW: a red Sign in when
 * nobody is signed in, the name with PRO or FREE when someone is. A click
 * opens the sign-in panel or the account card.
 *
 * Approved mockup: suite.hardwavestudios.com/daw-account-mockup/. Its two
 * questions went the way it recommends: signing in happens here in the DAW,
 * and the card shows the plan only, with no upgrade button.
 *
 * The password is typed here and handed to the backend once; the token the
 * site answers with stays in Rust and in the shared token file.
 */
export function AccountMenu() {
  useLanguage()
  const available = useAccountStore(s => s.available)
  const status = useAccountStore(s => s.status)
  const lastAccount = useAccountStore(s => s.lastAccount)
  const open = useAccountStore(s => s.open)
  const setOpen = useAccountStore(s => s.setOpen)
  const chipRef = useRef<HTMLButtonElement>(null)

  useEffect(() => watchAccount(), [])

  // Offline, the account last seen stays on show.
  const account: Account | null =
    status?.state === 'signedIn' ? status
      : status?.state === 'unreachable' ? lastAccount
        : null

  let chip: React.ReactNode
  if (account) {
    chip = (
      <>
        <span className="hw-acct-av" aria-hidden>{initialsOf(account.name)}</span>
        <span className="name">{account.name}</span>
        <PlanTag plan={account.plan} />
      </>
    )
  } else if (status === null || status.state === 'unreachable') {
    chip = t('Account')
  } else {
    chip = t('Sign in')
  }
  const signedOut = !account && (status?.state === 'signedOut' || status?.state === 'ended')

  if (!available) return null
  return (
    <>
      <button
        ref={chipRef}
        type="button"
        className={`hw-acct${signedOut ? ' out' : ''}${!account && !signedOut ? ' quiet' : ''}`}
        onClick={() => setOpen(!open)}
        aria-haspopup="dialog"
        aria-expanded={open}
        data-hint={account ? t('Your Hardwave account') : t('Sign in to your Hardwave account')}
      >
        {chip}
      </button>
      {open && (
        <Popover anchor={chipRef} onClose={() => setOpen(false)}>
          {account
            ? <AccountCard account={account} offline={status?.state === 'unreachable'} />
            : status?.state === 'unreachable'
              ? <CouldNotCheck />
              : <SignInPanel status={status} />}
        </Popover>
      )}
    </>
  )
}

function PlanTag({ plan }: { plan: Plan }) {
  if (plan.kind === 'pro') return <span className="hw-acct-tag pro">PRO</span>
  if (plan.kind === 'free') return <span className="hw-acct-tag free">{t('FREE')}</span>
  return null
}

/** Drawn on the page body, under the chip's right edge, above every panel. */
function Popover({ anchor, onClose, children }: {
  anchor: React.RefObject<HTMLElement | null>
  onClose: () => void
  children: React.ReactNode
}) {
  const ref = useRef<HTMLDivElement>(null)
  const [pos, setPos] = useState<{ top: number; right: number } | null>(null)

  useLayoutEffect(() => {
    const place = () => {
      const r = anchor.current?.getBoundingClientRect()
      if (r) setPos({ top: r.bottom + 6, right: Math.max(8, window.innerWidth - r.right) })
    }
    place()
    window.addEventListener('resize', place)
    return () => window.removeEventListener('resize', place)
  }, [anchor])

  useEffect(() => {
    const onDown = (e: MouseEvent) => {
      const target = e.target as Node
      if (ref.current?.contains(target) || anchor.current?.contains(target)) return
      onClose()
    }
    const onKey = (e: KeyboardEvent) => {
      if (e.key === 'Escape') { e.stopPropagation(); onClose() }
    }
    document.addEventListener('mousedown', onDown, true)
    document.addEventListener('keydown', onKey, true)
    return () => {
      document.removeEventListener('mousedown', onDown, true)
      document.removeEventListener('keydown', onKey, true)
    }
  }, [anchor, onClose])

  if (!pos) return null
  return createPortal(
    <div ref={ref} className="hw-acct-pop" role="dialog" style={{ top: pos.top, right: pos.right }}>
      {children}
    </div>,
    document.body,
  )
}

function SignInPanel({ status }: { status: AccountStatus | null }) {
  const busy = useAccountStore(s => s.busy)
  const error = useAccountStore(s => s.error)
  const signIn = useAccountStore(s => s.signIn)
  const clearError = useAccountStore(s => s.clearError)
  const openPage = useAccountStore(s => s.openPage)
  const [email, setEmail] = useState('')
  const [password, setPassword] = useState('')
  const [why, setWhy] = useState(false)
  const emailRef = useRef<HTMLInputElement>(null)

  useEffect(() => { emailRef.current?.focus() }, [])

  const submit = async (e: React.FormEvent) => {
    e.preventDefault()
    if (busy) return
    const ok = await signIn(email, password)
    // The password is not kept a moment longer than the request needs it.
    setPassword('')
    if (ok) setEmail('')
  }

  return (
    <form onSubmit={submit} aria-label={t('Sign in to Hardwave')}>
      <h3>{t('Sign in to Hardwave')}</h3>
      {status?.state === 'ended'
        ? <p className="warn">{t('Your sign-in has run out or was ended on the site. Sign in again.')}</p>
        : <p>{t('The same account as the Suite and the plug-ins. Signing in here signs the plug-ins in too.')}</p>}
      <label className="hw-acct-field">
        <span>{t('E-mail')}</span>
        <input
          ref={emailRef}
          type="email"
          autoComplete="username"
          spellCheck={false}
          value={email}
          onChange={e => { setEmail(e.target.value); if (error) clearError() }}
          placeholder="you@example.com"
        />
      </label>
      <label className="hw-acct-field">
        <span>{t('Password')}</span>
        <input
          type="password"
          autoComplete="current-password"
          value={password}
          onChange={e => { setPassword(e.target.value); if (error) clearError() }}
        />
      </label>
      {error && <div className="err" role="alert">{t(error)}</div>}
      <button type="submit" className="hw-acct-btn primary" disabled={busy}>
        {busy ? t('Signing in…') : t('Sign in')}
      </button>
      <button type="button" className="hw-acct-btn" onClick={() => openPage('register')}>
        {t('Make an account (opens the site)')}
      </button>
      <div className="hw-acct-links">
        <button type="button" className="hw-acct-link" onClick={() => openPage('forgot-password')}>
          {t('Forgot your password?')}
        </button>
        <button type="button" className="hw-acct-link" aria-expanded={why} onClick={() => setWhy(v => !v)}>
          {t('Why sign in?')}
        </button>
      </div>
      {why && (
        <p className="note">
          {t('The DAW and the Hardwave plug-ins in it are free, with or without an account. An account lets you join a room someone else hosts. Pro adds what runs on our servers, like hosting a room and stem separation, and the plug-ins in other DAWs.')}
        </p>
      )}
    </form>
  )
}

function planLine(plan: Plan): React.ReactNode {
  if (plan.kind === 'free') return t('Free')
  if (plan.kind === 'unknown') return t('Could not check')
  const label = plan.via === 'trial' ? t('Pro (trial)') : plan.via === 'staff' ? t('Pro (staff)') : t('Pro')
  if (plan.until) {
    const day = new Date(`${plan.until}T00:00:00`)
    const shown = isNaN(day.getTime())
      ? plan.until
      : day.toLocaleDateString(undefined, { day: 'numeric', month: 'short', year: 'numeric' })
    return tf('{plan} until {day}', { plan: label, day: shown })
  }
  return <>{label} <span className="ok" aria-hidden>●</span></>
}

function AccountCard({ account, offline }: { account: Account; offline: boolean }) {
  const busy = useAccountStore(s => s.busy)
  const error = useAccountStore(s => s.error)
  const signOut = useAccountStore(s => s.signOut)
  const openPage = useAccountStore(s => s.openPage)
  const { plan } = account

  return (
    <div aria-label={t('Your Hardwave account')}>
      <div className="hw-acct-who">
        <span className="hw-acct-av big" aria-hidden>{initialsOf(account.name)}</span>
        <div>
          <b>{account.name}</b>
          <small>{account.email}</small>
        </div>
      </div>
      <div className="hw-acct-kv"><span>{t('Plan')}</span><b>{planLine(plan)}</b></div>
      <div className="hw-acct-kv">
        <span>{t('Pro plug-ins')}</span>
        <b>{plan.kind === 'pro' ? t('all unlocked') : plan.kind === 'free' ? t('free in this DAW') : t('Could not check')}</b>
      </div>
      <div className="hw-acct-kv"><span>{t('Signed in until')}</span><b>{t('renews by itself')}</b></div>
      {offline && (
        <p className="note warn">{t('Could not reach hardwavestudios.com, so this is what it said last.')}</p>
      )}
      {error && <div className="err" role="alert">{t(error)}</div>}
      <button type="button" className="hw-acct-btn" onClick={() => openPage('account')}>
        {t('Manage your account (opens the site)')}
      </button>
      <button type="button" className="hw-acct-btn" disabled={busy} onClick={() => void signOut()}>
        {busy ? t('Signing out…') : t('Sign out')}
      </button>
      <p className="note">{t('Signs out the DAW and the plug-ins on this computer. The Suite has its own Sign out.')}</p>
    </div>
  )
}

/** Signed in on this computer, but the site could not be asked yet. */
function CouldNotCheck() {
  const checking = useAccountStore(s => s.checking)
  const busy = useAccountStore(s => s.busy)
  const refresh = useAccountStore(s => s.refresh)
  const signOut = useAccountStore(s => s.signOut)
  return (
    <div>
      <h3>{t('Signed in')}</h3>
      <p>{t('This computer is signed in, but hardwavestudios.com could not be reached to show the account.')}</p>
      <button type="button" className="hw-acct-btn primary" disabled={checking} onClick={() => void refresh({ force: true })}>
        {checking ? t('Checking…') : t('Try again')}
      </button>
      <button type="button" className="hw-acct-btn" disabled={busy} onClick={() => void signOut()}>
        {t('Sign out')}
      </button>
    </div>
  )
}
