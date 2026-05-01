import { useEffect, useRef, useState } from 'react'
import anime from 'animejs'
import { HwLogo } from './HwLogo'

interface SplashScreenProps {
  dataReady: boolean
  onFinished: () => void
}

export function SplashScreen({ dataReady, onFinished }: SplashScreenProps) {
  const containerRef = useRef<HTMLDivElement>(null)
  const logoRef = useRef<HTMLDivElement>(null)
  const shineRef = useRef<HTMLDivElement>(null)
  const titleRef = useRef<HTMLHeadingElement>(null)
  const subtitleRef = useRef<HTMLParagraphElement>(null)
  const barRef = useRef<HTMLDivElement>(null)
  const barFillRef = useRef<HTMLDivElement>(null)
  const particlesRef = useRef<HTMLDivElement>(null)
  const glow1Ref = useRef<HTMLDivElement>(null)
  const glow2Ref = useRef<HTMLDivElement>(null)
  const ring1Ref = useRef<HTMLDivElement>(null)
  const ring2Ref = useRef<HTMLDivElement>(null)
  const exitCalledRef = useRef(false)
  const [animDone, setAnimDone] = useState(false)

  useEffect(() => {
    const tl = anime.timeline({ easing: 'easeOutExpo' })

    // Background glows fade in (breathing is now CSS-driven)
    tl.add({
      targets: [glow1Ref.current, glow2Ref.current],
      opacity: [0, 1],
      scale: [0.5, 1],
      duration: 1200,
      complete: () => {
        glow1Ref.current?.classList.add('glow-breathe-1')
        glow2Ref.current?.classList.add('glow-breathe-2')
      },
    }, 0)

    // Particles
    if (particlesRef.current) {
      anime({
        targets: particlesRef.current.children,
        translateY: ['100vh', '-10vh'],
        opacity: [0, 1, 0],
        scale: [0, 1, 0],
        duration: () => 3000 + Math.random() * 3000,
        delay: () => Math.random() * 2000,
        easing: 'easeInOutQuad',
        loop: true,
      })
    }

    // Logo slam in
    tl.add({
      targets: logoRef.current,
      scale: [0, 1.15, 1],
      rotate: ['-15deg', '3deg', '0deg'],
      opacity: [0, 1],
      duration: 900,
      easing: 'easeOutElastic(1, 0.6)',
    }, 200)

    // Logo shadow — red glow
    tl.add({
      targets: logoRef.current,
      boxShadow: [
        '0 0 0px rgba(220,38,38,0), 0 0 0px rgba(185,28,28,0)',
        '0 0 80px rgba(220,38,38,0.5), 0 0 160px rgba(185,28,28,0.3)',
        '0 0 40px rgba(220,38,38,0.25), 0 0 80px rgba(185,28,28,0.15)',
      ],
      duration: 800,
      easing: 'easeOutQuad',
    }, 800)

    // Shine sweep
    tl.add({
      targets: shineRef.current,
      translateX: ['-200%', '200%'],
      duration: 700,
      easing: 'easeInOutQuad',
    }, 1000)

    // Expanding rings
    tl.add({
      targets: ring1Ref.current,
      scale: [0.8, 4],
      opacity: [0.5, 0],
      borderWidth: ['2px', '0px'],
      duration: 1200,
      easing: 'easeOutCubic',
    }, 900)
    tl.add({
      targets: ring2Ref.current,
      scale: [0.8, 3.5],
      opacity: [0.3, 0],
      borderWidth: ['2px', '0px'],
      duration: 1200,
      easing: 'easeOutCubic',
    }, 1050)

    // Title
    tl.add({
      targets: titleRef.current,
      translateY: [40, 0],
      opacity: [0, 1],
      filter: ['blur(12px)', 'blur(0px)'],
      duration: 700,
      easing: 'easeOutCubic',
    }, 1100)

    // Subtitle
    tl.add({
      targets: subtitleRef.current,
      translateY: [25, 0],
      opacity: [0, 0.5],
      duration: 600,
      easing: 'easeOutCubic',
    }, 1300)

    // Loading bar appear
    tl.add({
      targets: barRef.current,
      opacity: [0, 1],
      scaleX: [0, 1],
      duration: 500,
      easing: 'easeOutCubic',
    }, 1400)

    // Loading bar: fill to 60%
    tl.add({
      targets: barFillRef.current,
      width: ['0%', '60%'],
      duration: 800,
      easing: 'easeOutCubic',
    }, 1600)

    // Slowly creep to 85%
    tl.add({
      targets: barFillRef.current,
      width: '85%',
      duration: 1200,
      easing: 'easeOutSine',
      complete: () => setAnimDone(true),
    }, 2400)

    return () => { tl.pause() }
  }, [])

  // Exit when both intro animation done AND data ready
  useEffect(() => {
    if (animDone && dataReady && !exitCalledRef.current) {
      exitCalledRef.current = true

      const tl = anime.timeline({ easing: 'easeOutCubic' })

      // Fill bar to 100%
      tl.add({
        targets: barFillRef.current,
        width: '100%',
        duration: 400,
        easing: 'easeOutQuart',
      }, 0)

      // Fade out
      tl.add({
        targets: containerRef.current,
        opacity: [1, 0],
        scale: [1, 1.05],
        duration: 500,
        easing: 'easeInCubic',
        complete: onFinished,
      }, 500)
    }
  }, [animDone, dataReady, onFinished])

  // Particle colors — alternating red shades
  const particles = Array.from({ length: 12 }).map((_, i) => ({
    width: 2 + Math.random() * 3,
    left: Math.random() * 100,
    bg: i % 2 === 0
      ? 'linear-gradient(135deg, #DC2626, #B91C1C)'
      : 'linear-gradient(135deg, #EF4444, #DC2626)',
  }))

  return (
    <div ref={containerRef} style={{
      position: 'fixed', inset: 0, zIndex: 50,
      display: 'flex', alignItems: 'center', justifyContent: 'center',
      background: '#08080c', overflow: 'hidden',
    }}>
      {/* CSS keyframes for glow breathing (GPU-composited, no JS) */}
      <style>{`
        @keyframes glowBreathe1 {
          0%, 100% { transform: translate(-50%, -50%) translateZ(0) scale(1); opacity: 0.8; }
          50% { transform: translate(-50%, -50%) translateZ(0) scale(1.2); opacity: 1; }
        }
        @keyframes glowBreathe2 {
          0%, 100% { transform: translate(-40%, -60%) translateZ(0) scale(1); opacity: 0.7; }
          50% { transform: translate(-40%, -60%) translateZ(0) scale(1.15); opacity: 1; }
        }
        .glow-breathe-1 {
          animation: glowBreathe1 3s ease-in-out infinite;
        }
        .glow-breathe-2 {
          animation: glowBreathe2 3.5s ease-in-out 0.5s infinite;
        }
      `}</style>

      {/* Particles */}
      <div ref={particlesRef} style={{ position: 'absolute', inset: 0, pointerEvents: 'none' }}>
        {particles.map((p, i) => (
          <div
            key={i}
            style={{
              position: 'absolute',
              width: p.width,
              height: p.width,
              left: `${p.left}%`,
              bottom: 0,
              borderRadius: '50%',
              background: p.bg,
              opacity: 0,
              willChange: 'transform, opacity',
              transform: 'translateZ(0)',
            }}
          />
        ))}
      </div>

      {/* Background glows */}
      <div ref={glow1Ref} style={{
        position: 'absolute', top: '50%', left: '50%',
        transform: 'translate(-50%, -50%) translateZ(0)',
        width: 500, height: 500, borderRadius: '50%',
        background: 'rgba(220, 38, 38, 0.08)',
        filter: 'blur(120px)', opacity: 0,
        willChange: 'transform, opacity',
      }} />
      <div ref={glow2Ref} style={{
        position: 'absolute', top: '50%', left: '50%',
        transform: 'translate(-40%, -60%) translateZ(0)',
        width: 400, height: 400, borderRadius: '50%',
        background: 'rgba(185, 28, 28, 0.05)',
        filter: 'blur(120px)', opacity: 0,
        willChange: 'transform, opacity',
      }} />

      {/* Center content */}
      <div style={{ position: 'relative', display: 'flex', flexDirection: 'column', alignItems: 'center' }}>
        <div ref={ring1Ref} style={{
          position: 'absolute', width: 96, height: 96, borderRadius: '50%',
          border: '2px solid rgba(220, 38, 38, 0.4)', opacity: 0, top: 0,
        }} />
        <div ref={ring2Ref} style={{
          position: 'absolute', width: 96, height: 96, borderRadius: '50%',
          border: '2px solid rgba(220, 38, 38, 0.2)', opacity: 0, top: 0,
        }} />

        {/* Logo with shine sweep */}
        <div ref={logoRef} style={{
          position: 'relative', overflow: 'hidden',
          borderRadius: 24, width: 96, height: 96, opacity: 0,
        }}>
          <HwLogo size={96} />
          <div ref={shineRef} style={{
            position: 'absolute', inset: 0,
            background: 'linear-gradient(105deg, transparent 40%, rgba(255,255,255,0.25) 50%, transparent 60%)',
          }} />
        </div>

        <h1 ref={titleRef} style={{
          fontSize: 32, fontWeight: 700, color: '#FFF',
          marginTop: 24, opacity: 0,
          fontFamily: "'Space Grotesk', Inter, ui-sans-serif, system-ui, sans-serif",
          letterSpacing: '-0.02em',
        }}>
          Hardwave DAW
        </h1>
        <p ref={subtitleRef} style={{
          fontSize: 12, color: '#a1a1a6',
          marginTop: 10, opacity: 0,
          fontFamily: "'JetBrains Mono', ui-monospace, SFMono-Regular, Menlo, monospace",
          letterSpacing: '0.08em',
          textTransform: 'uppercase',
          fontWeight: 500,
        }}>
          Hard hours, harder hits.
        </p>

        <div ref={barRef} style={{
          marginTop: 32, width: 200, height: 2,
          background: 'rgba(255,255,255,0.06)',
          borderRadius: 2, overflow: 'hidden', opacity: 0,
        }}>
          <div ref={barFillRef} style={{
            height: '100%', width: '0%',
            background: 'linear-gradient(90deg, #B91C1C, #DC2626)',
            borderRadius: 2,
          }} />
        </div>
      </div>
    </div>
  )
}
