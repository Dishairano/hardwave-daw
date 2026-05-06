import { useEffect, useRef, useState } from 'react'

export interface MenuItem {
  label: string
  shortcut?: string
  action?: () => void
  separator?: boolean
  disabled?: boolean
  submenu?: MenuItem[]
}

export interface MenuDef {
  label: string
  items: MenuItem[]
}

interface HwTopMenuProps {
  menus: MenuDef[]
}

export function HwTopMenu({ menus }: HwTopMenuProps) {
  const [openMenu, setOpenMenu] = useState<number | null>(null)
  const [openSubmenu, setOpenSubmenu] = useState<number | null>(null)
  const barRef = useRef<HTMLDivElement>(null)

  useEffect(() => {
    if (openMenu === null) return
    const onMouseDown = (e: MouseEvent) => {
      if (barRef.current && !barRef.current.contains(e.target as Node)) {
        setOpenMenu(null)
        setOpenSubmenu(null)
      }
    }
    const onKey = (e: KeyboardEvent) => {
      if (e.key === 'Escape') {
        setOpenMenu(null)
        setOpenSubmenu(null)
      }
    }
    window.addEventListener('mousedown', onMouseDown)
    window.addEventListener('keydown', onKey)
    return () => {
      window.removeEventListener('mousedown', onMouseDown)
      window.removeEventListener('keydown', onKey)
    }
  }, [openMenu])

  return (
    <div className="fl-menu" ref={barRef}>
      {menus.map((menu, idx) => (
        <span
          key={menu.label}
          className={openMenu === idx ? 'open' : ''}
          onMouseDown={(e) => {
            e.stopPropagation()
            setOpenMenu(openMenu === idx ? null : idx)
            setOpenSubmenu(null)
          }}
          onMouseEnter={() => {
            if (openMenu !== null && openMenu !== idx) {
              setOpenMenu(idx)
              setOpenSubmenu(null)
            }
          }}
        >
          {menu.label}
          {openMenu === idx && (
            <div
              className="fl-menu-dropdown"
              onMouseDown={(e) => e.stopPropagation()}
            >
              {menu.items.map((item, i) =>
                item.separator ? (
                  <div key={i} className="fl-menu-sep" />
                ) : (
                  <MenuRow
                    key={i}
                    item={item}
                    isSubmenuOpen={openSubmenu === i}
                    onEnterSubmenu={() => item.submenu && setOpenSubmenu(i)}
                    onLeaveSubmenu={() => setOpenSubmenu((s) => (s === i ? null : s))}
                    onClickItem={() => {
                      if (item.disabled) return
                      if (item.submenu) return
                      item.action?.()
                      setOpenMenu(null)
                      setOpenSubmenu(null)
                    }}
                    onClickSubmenuItem={() => {
                      setOpenMenu(null)
                      setOpenSubmenu(null)
                    }}
                  />
                ),
              )}
            </div>
          )}
        </span>
      ))}
    </div>
  )
}

interface MenuRowProps {
  item: MenuItem
  isSubmenuOpen: boolean
  onEnterSubmenu: () => void
  onLeaveSubmenu: () => void
  onClickItem: () => void
  onClickSubmenuItem: () => void
}

function MenuRow({ item, isSubmenuOpen, onEnterSubmenu, onLeaveSubmenu, onClickItem, onClickSubmenuItem }: MenuRowProps) {
  return (
    <div
      className="fl-menu-row-host"
      onMouseEnter={() => {
        if (item.submenu) onEnterSubmenu()
        else onLeaveSubmenu()
      }}
    >
      <div
        className={`fl-menu-row${item.disabled ? ' disabled' : ''}${isSubmenuOpen ? ' submenu-open' : ''}`}
        onClick={onClickItem}
      >
        <span className="lbl">{item.label}</span>
        {item.shortcut && <span className="kbd">{item.shortcut}</span>}
        {item.submenu && <span className="arrow">▶</span>}
      </div>

      {item.submenu && isSubmenuOpen && (
        <div className="fl-menu-dropdown submenu">
          {item.submenu.map((sub, si) =>
            sub.separator ? (
              <div key={si} className="fl-menu-sep" />
            ) : (
              <div
                key={si}
                className={`fl-menu-row${sub.disabled ? ' disabled' : ''}`}
                onClick={() => {
                  if (sub.disabled || !sub.action) return
                  sub.action()
                  onClickSubmenuItem()
                }}
              >
                <span className="lbl">{sub.label}</span>
                {sub.shortcut && <span className="kbd">{sub.shortcut}</span>}
              </div>
            ),
          )}
        </div>
      )}
    </div>
  )
}
