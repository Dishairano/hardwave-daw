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

    // Background glows fade in
    tl.add({
      targets: [glow1Ref.current, glow2Ref.current],
      opacity: [0, 1],
      scale: [0.5, 1],
      duration: 1200,
    }, 0)

    // Glow breathing
    anime({
      targets: glow1Ref.current,
      scale: [1, 1.2, 1],
      opacity: [0.8, 1, 0.8],
      duration: 3000,
      easing: 'easeInOutSine',
      loop: true,
    })
    anime({
      targets: glow2Ref.current,
      scale: [1, 1.15, 1],
      opacity: [0.7, 1, 0.7],
      duration: 3500,
      easing: 'easeInOutSine',
      loop: true,
      delay: 500,
    })

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

    // Logo shadow — purple glow
    tl.add({
      targets: logoRef.current,
      boxShadow: [
        '0 0 0px rgba(155,109,255,0), 0 0 0px rgba(123,90,192,0)',
        '0 0 80px rgba(155,109,255,0.5), 0 0 160px rgba(123,90,192,0.3)',
        '0 0 40px rgba(155,109,255,0.25), 0 0 80px rgba(123,90,192,0.15)',
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

  // Particle colors — alternating purple shades
  const particles = Array.from({ length: 25 }).map((_, i) => ({
    width: 2 + Math.random() * 3,
    left: Math.random() * 100,
    bg: i % 2 === 0
      ? 'linear-gradient(135deg, #9B6DFF, #7B5AC0)'
      : 'linear-gradient(135deg, #B48EFF, #9B6DFF)',
  }))

  return (
    <div ref={containerRef} style={{
      position: 'fixed', inset: 0, zIndex: 50,
      display: 'flex', alignItems: 'center', justifyContent: 'center',
      background: '#0c0c10', overflow: 'hidden',
    }}>
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
            }}
          />
        ))}
      </div>

      {/* Background glows */}
      <div ref={glow1Ref} style={{
        position: 'absolute', top: '50%', left: '50%',
        transform: 'translate(-50%, -50%)',
        width: 500, height: 500, borderRadius: '50%',
        background: 'rgba(155, 109, 255, 0.08)',
        filter: 'blur(120px)', opacity: 0,
      }} />
      <div ref={glow2Ref} style={{
        position: 'absolute', top: '50%', left: '50%',
        transform: 'translate(-40%, -60%)',
        width: 400, height: 400, borderRadius: '50%',
        background: 'rgba(123, 90, 192, 0.05)',
        filter: 'blur(120px)', opacity: 0,
      }} />

      {/* Center content */}
      <div style={{ position: 'relative', display: 'flex', flexDirection: 'column', alignItems: 'center' }}>
        <div ref={ring1Ref} style={{
          position: 'absolute', width: 96, height: 96, borderRadius: '50%',
          border: '2px solid rgba(155, 109, 255, 0.4)', opacity: 0, top: 0,
        }} />
        <div ref={ring2Ref} style={{
          position: 'absolute', width: 96, height: 96, borderRadius: '50%',
          border: '2px solid rgba(155, 109, 255, 0.2)', opacity: 0, top: 0,
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
          fontSize: 30, fontWeight: 700, color: '#FFF',
          marginTop: 24, opacity: 0,
          fontFamily: "'Segoe UI', -apple-system, sans-serif",
        }}>
          Hardwave DAW
        </h1>
        <p ref={subtitleRef} style={{
          fontSize: 14, color: '#58585F',
          marginTop: 8, opacity: 0,
          fontFamily: "'Segoe UI', -apple-system, sans-serif",
        }}>
          Digital Audio Workstation
        </p>

        <div ref={barRef} style={{
          marginTop: 32, width: 200, height: 2,
          background: 'rgba(255,255,255,0.06)',
          borderRadius: 2, overflow: 'hidden', opacity: 0,
        }}>
          <div ref={barFillRef} style={{
            height: '100%', width: '0%',
            background: 'linear-gradient(90deg, #7B5AC0, #9B6DFF)',
            borderRadius: 2,
          }} />
        </div>
      </div>
    </div>
  )
}
