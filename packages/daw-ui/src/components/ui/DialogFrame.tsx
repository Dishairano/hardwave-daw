import type { ReactNode } from 'react'

/**
 * The one frame every modal dialog uses: a dimmed backdrop, a header with
 * the title, an optional line under it and the close button, the body, and
 * an optional footer whose buttons sit on the right. Styles: rework.css
 * (.hw-dialog*).
 */
export function DialogFrame({ title, subtitle, onClose, headerActions, footer, width = 720, height, children }: {
  title: string
  subtitle?: string
  onClose: () => void
  /** Extra buttons in the header, left of the close button. */
  headerActions?: ReactNode
  footer?: ReactNode
  width?: number
  /** A fixed height (for example '80vh'); by default the frame fits its content. */
  height?: number | string
  children: ReactNode
}) {
  return (
    <div
      className="hw-dialog-backdrop"
      onMouseDown={(e) => { if (e.target === e.currentTarget) onClose() }}
    >
      <div className="hw-dialog" style={{ width, height }} role="dialog" aria-label={title}>
        <div className="hw-dialog-head">
          <div className="hw-dialog-titles">
            <b>{title}</b>
            {subtitle && <span>{subtitle}</span>}
          </div>
          {headerActions}
          <button type="button" className="hw-dialog-x" onClick={onClose} title="Close" aria-label="Close">
            <svg width="12" height="12" viewBox="0 0 12 12"><path d="M2.5 2.5l7 7M9.5 2.5l-7 7" stroke="currentColor" strokeWidth="1.5" strokeLinecap="round" /></svg>
          </button>
        </div>
        <div className="hw-dialog-body">{children}</div>
        {footer && <div className="hw-dialog-foot">{footer}</div>}
      </div>
    </div>
  )
}
