import { useEffect, useState } from 'react'
import { hw } from '../theme'
import { t, useLanguage } from '../i18n'

const SKIP_KEY = 'hardwave.daw.welcomeSkip'

export function shouldSkipWelcome() {
  try { return localStorage.getItem(SKIP_KEY) === '1' } catch { return false }
}

interface Props {
  recentProjects: string[]
  onOpenRecent: (path: string) => void
  onNewProject: () => void
  onOpenProject: () => void
  onOpenSampleProject: () => void
  onOpenAudioSettings: () => void
  onDismiss: () => void
}

export function WelcomeScreen({
  recentProjects,
  onOpenRecent,
  onNewProject,
  onOpenProject,
  onOpenSampleProject,
  onOpenAudioSettings,
  onDismiss,
}: Props) {
  useLanguage()
  const [mounted, setMounted] = useState(false)
  const [skipNext, setSkipNext] = useState<boolean>(() => shouldSkipWelcome())
  useEffect(() => { setMounted(true) }, [])

  // The key sits in the sentence wherever the language puts it.
  const [pressBefore, pressAfter] = t('Press {key} anytime for shortcuts').split('{key}')

  const dismiss = () => {
    try { localStorage.setItem(SKIP_KEY, skipNext ? '1' : '0') } catch {}
    onDismiss()
  }

  return (
    <div style={{
      position: 'fixed',
      inset: 0,
      zIndex: 5000,
      background: `radial-gradient(circle at 50% 30%, rgba(220,38,38,0.08) 0%, ${hw.bg} 60%)`,
      display: 'flex',
      alignItems: 'center',
      justifyContent: 'center',
      opacity: mounted ? 1 : 0,
      transition: 'opacity 0.25s',
    }}>
      <div style={{
        width: 720,
        maxWidth: '90vw',
        background: hw.bgPanel,
        backdropFilter: hw.blur.md,
        border: `1px solid ${hw.border}`,
        borderRadius: hw.radius.lg,
        padding: 32,
        boxShadow: hw.glowRed,
      }}>
        <div style={{
          fontSize: 28,
          fontWeight: 700,
          color: hw.textBright,
          marginBottom: 4,
          letterSpacing: -0.5,
        }}>
          Hardwave DAW
        </div>
        <div style={{ fontSize: 13, color: hw.textMuted, marginBottom: 28 }}>
          {t('Start a new project or open a recent one.')}
        </div>

        <div style={{ display: 'grid', gridTemplateColumns: '1fr 1fr', gap: 28 }}>
          <div>
            <div style={{ fontSize: 11, color: hw.textFaint, textTransform: 'uppercase', letterSpacing: 0.8, marginBottom: 10 }}>
              {t('Get started')}
            </div>
            <button
              onClick={() => { onNewProject(); dismiss() }}
              style={actionBtn(true)}
            >
              {t('New project')}
            </button>
            <button
              onClick={() => { onOpenProject(); dismiss() }}
              style={actionBtn(false)}
            >
              {t('Open project…')}
            </button>
            <button
              onClick={() => { onOpenSampleProject(); dismiss() }}
              style={actionBtn(false)}
              title={t('A hardstyle song at 150 BPM with a kick that plays straight away. New project offers rawstyle, frenchcore and uptempo too.')}
            >
              {t('Start from a hardstyle song')}
            </button>
            <button
              onClick={() => { onOpenAudioSettings() }}
              style={actionBtn(false)}
              title={t('Pick an output device, sample rate and buffer size')}
            >
              {t('Configure audio device…')}
            </button>
            <button
              onClick={dismiss}
              style={{ ...actionBtn(false), marginTop: 18 }}
            >
              {t('Close this screen')}
            </button>
          </div>

          <div>
            <div style={{ fontSize: 11, color: hw.textFaint, textTransform: 'uppercase', letterSpacing: 0.8, marginBottom: 10 }}>
              {t('Recent projects')}
            </div>
            {recentProjects.length === 0 ? (
              <div style={{ fontSize: 12, color: hw.textMuted, padding: '16px 0' }}>
                {t('No recent projects yet.')}
              </div>
            ) : (
              <div style={{ maxHeight: 260, overflowY: 'auto' }}>
                {recentProjects.slice(0, 8).map(p => {
                  const name = p.split(/[\\/]/).pop()?.replace('.hwp', '') || p
                  return (
                    <button
                      key={p}
                      onClick={() => { onOpenRecent(p); dismiss() }}
                      style={recentBtn}
                      title={p}
                    >
                      <div style={{ fontSize: 13, color: hw.textPrimary, fontWeight: 500 }}>{name}</div>
                      <div style={{ fontSize: 10, color: hw.textMuted, marginTop: 2, overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap' }}>
                        {p}
                      </div>
                    </button>
                  )
                })}
              </div>
            )}
          </div>
        </div>

        <div style={{
          marginTop: 24, paddingTop: 16,
          borderTop: `1px solid ${hw.border}`,
        }}>
          <div style={{
            fontSize: 11, color: hw.textFaint,
            textTransform: 'uppercase', letterSpacing: 0.8, marginBottom: 10,
          }}>
            {t('Quick tour')}
          </div>
          <div style={{
            display: 'grid', gridTemplateColumns: '1fr 1fr', gap: '6px 18px',
            fontSize: 11, color: hw.textSecondary,
          }}>
            <PanelHint shortcut="F5" label={t('Playlist')} hint={t('Arrange clips on a timeline')} />
            <PanelHint shortcut="F6" label={t('Channel Rack')} hint={t('Step-sequence drums and loops')} />
            <PanelHint shortcut="F7" label={t('Piano Roll')} hint={t('Draw and edit MIDI notes')} />
            <PanelHint shortcut="F8" label={t('Browser')} hint={t('Find samples and plugins')} />
            <PanelHint shortcut="F9" label={t('Mixer')} hint={t('Levels, routing and FX chains')} />
            <PanelHint shortcut="?" label={t('Shortcuts')} hint={t('Full keyboard cheat sheet')} />
          </div>
        </div>

        <div style={{
          marginTop: 24, paddingTop: 16,
          borderTop: `1px solid ${hw.border}`,
        }}>
          <div style={{
            fontSize: 11, color: hw.textFaint,
            textTransform: 'uppercase', letterSpacing: 0.8, marginBottom: 10,
          }}>
            {t('Documentation & help')}
          </div>
          <div style={{
            display: 'flex', gap: 18, flexWrap: 'wrap',
            fontSize: 12,
          }}>
            <DocLink
              href="https://github.com/Dishairano/hardwave-daw#readme"
              label={t('Read the docs')}
              hint={t('Project README on GitHub')}
            />
            <DocLink
              href="https://github.com/Dishairano/hardwave-daw/releases"
              label={t('Release notes')}
              hint={t('Changelog for every version')}
            />
            <DocLink
              href="https://github.com/Dishairano/hardwave-daw/issues"
              label={t('Report an issue')}
              hint={t('Bug reports and feature requests')}
            />
          </div>
        </div>

        <div style={{
          marginTop: 20, display: 'flex', alignItems: 'center', gap: 8,
          fontSize: 11, color: hw.textMuted,
        }}>
          <label style={{ display: 'flex', alignItems: 'center', gap: 6, cursor: 'pointer' }}>
            <input
              type="checkbox"
              checked={skipNext}
              onChange={e => setSkipNext(e.target.checked)}
              style={{ accentColor: hw.accent }}
            />
            {t("Don't show this on startup")}
          </label>
          <div style={{ flex: 1 }} />
          <span style={{ fontSize: 10, color: hw.textFaint }}>
            {pressBefore}<kbd style={kbdStyle}>?</kbd>{pressAfter}
          </span>
        </div>
      </div>
    </div>
  )
}

function DocLink({ href, label, hint }: { href: string; label: string; hint: string }) {
  return (
    <a
      href={href}
      target="_blank"
      rel="noopener noreferrer"
      style={{
        display: 'flex', flexDirection: 'column', gap: 2,
        padding: '8px 12px', textDecoration: 'none',
        background: hw.bgElevated, border: `1px solid ${hw.border}`,
        borderRadius: hw.radius.md, minWidth: 160,
        transition: 'background 0.1s, border-color 0.1s',
      }}
      onMouseEnter={e => {
        (e.currentTarget as HTMLElement).style.background = hw.accentDim
        ;(e.currentTarget as HTMLElement).style.borderColor = hw.accentGlow
      }}
      onMouseLeave={e => {
        (e.currentTarget as HTMLElement).style.background = hw.bgElevated
        ;(e.currentTarget as HTMLElement).style.borderColor = hw.border
      }}
    >
      <span style={{ fontSize: 12, fontWeight: 600, color: hw.textPrimary }}>{label} →</span>
      <span style={{ fontSize: 10, color: hw.textFaint }}>{hint}</span>
    </a>
  )
}

function PanelHint({ shortcut, label, hint }: { shortcut: string; label: string; hint: string }) {
  return (
    <div style={{ display: 'flex', alignItems: 'baseline', gap: 8 }}>
      <kbd style={kbdStyle}>{shortcut}</kbd>
      <span style={{ fontWeight: 600, color: hw.textPrimary }}>{label}</span>
      <span style={{ color: hw.textFaint, fontSize: 10 }}>{hint}</span>
    </div>
  )
}

const kbdStyle: React.CSSProperties = {
  display: 'inline-block',
  minWidth: 22,
  padding: '1px 5px',
  fontSize: 10,
  fontFamily: 'ui-monospace, Menlo, monospace',
  color: hw.accent,
  background: 'rgba(255,255,255,0.05)',
  border: `1px solid ${hw.border}`,
  borderRadius: 4,
  textAlign: 'center',
}

function actionBtn(primary: boolean): React.CSSProperties {
  return {
    display: 'block',
    width: '100%',
    padding: '10px 14px',
    marginBottom: 8,
    textAlign: 'left',
    fontSize: 13,
    fontWeight: 500,
    color: primary ? hw.textBright : hw.textPrimary,
    background: primary ? hw.accent : hw.bgElevated,
    border: `1px solid ${primary ? hw.accent : hw.border}`,
    borderRadius: hw.radius.md,
    cursor: 'pointer',
  }
}

const recentBtn: React.CSSProperties = {
  display: 'block',
  width: '100%',
  padding: '8px 10px',
  marginBottom: 4,
  textAlign: 'left',
  background: 'transparent',
  border: `1px solid transparent`,
  borderRadius: hw.radius.sm,
  cursor: 'pointer',
}
