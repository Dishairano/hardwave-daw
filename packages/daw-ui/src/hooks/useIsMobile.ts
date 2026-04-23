import { useEffect, useState } from 'react'

export const MOBILE_BREAKPOINT = 768

function compute(): boolean {
  if (typeof window === 'undefined') return false
  return window.innerWidth <= MOBILE_BREAKPOINT
}

export function useIsMobile(): boolean {
  const [isMobile, setIsMobile] = useState<boolean>(compute)
  useEffect(() => {
    const onResize = () => setIsMobile(compute())
    window.addEventListener('resize', onResize)
    window.addEventListener('orientationchange', onResize)
    return () => {
      window.removeEventListener('resize', onResize)
      window.removeEventListener('orientationchange', onResize)
    }
  }, [])
  return isMobile
}
