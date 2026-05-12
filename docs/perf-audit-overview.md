# Hardwave DAW — Performance Overview

**Date:** 2026-05-12
**Audience:** producers, beta testers, anyone who wants to know how the DAW is doing today without reading code

This is the plain-language version. The full engineering report with file
paths and code references lives at
[`docs/perf-audit.md`](https://github.com/Dishairano/hardwave-daw/blob/redesign-port-from-master/docs/perf-audit.md).

---

## In one paragraph

Hardwave DAW today handles small projects (10–50 tracks) smoothly, but
larger sessions (200+ tracks, many plug-ins) hit walls that don't need
new code to solve — most of the speed-ups have already been built but
are still hidden behind a switch. Three big patterns slow things down:
the audio engine works on every track all the time, even silent ones;
every fader move triggers a flood of "are we still in sync?" messages
between the audio side and the screen side; and the new, fast mixer
panel is opt-in, so most people still see the older slow one.

---

## What feels slow today

### 1. The new mixer is hidden

We spent weeks building a much faster mixer: it only draws the strips
you can actually see, paints level meters directly to the screen instead
of going through React, and uses springy GPU-accelerated scrolling. It
works correctly. **But it's off by default.** You have to go to
**Options → "Experimental — FL Wide 2 mixer"** to turn it on.

> **Why this matters:** the single biggest user-visible speed boost
> already exists in the code. Flipping one default would deliver it.
> Effort: ~15 minutes. Risk: very low — falling back to the old mixer is
> one click away.

### 2. The audio engine never rests

Even when a track is silent — no clips playing, no input, no automation —
the engine still runs the full effect chain on it (every compressor, EQ,
reverb, etc.) every fraction of a second. On a 500-track session that's
hundreds of plug-ins doing real work for sound that doesn't exist.

> **Why this matters:** Audio CPU stays high in big sessions regardless
> of how much is actually playing. Optimising this means silent tracks
> cost almost nothing — only the ones making sound count.
> Effort: ~4 hours engine work + careful testing for effects with tails
> (reverbs, delays).

### 3. Every fader move sends 500 questions

When you change the volume of one track in a 500-track session, the
display has no quick way to know what the new value should be, so it
asks the audio side for the full state of every track all over again.
That's about 500 little messages flying back and forth for one slider
move.

> **Why this matters:** This is why fader drags can stutter on big
> sessions. The fix is to make the audio side just tell the display
> "track 17 changed, here's the new value" instead of "everything
> changed, ask again about everything."
> Effort: ~6 hours.

### 4. The audio thread keeps asking for memory

Several thousand times every second, while music is playing, the audio
engine asks the operating system for fresh chunks of memory. This is
considered bad practice for real-time audio — it's a known source of
audio glitches under load.

> **Why this matters:** Today the system mostly gets away with it
> because we have modern fast hardware and conservative buffer sizes.
> But at low-latency settings (small buffers used for live recording)
> this is the most likely cause of clicks and pops.
> Effort: ~6 hours engine work.

### 5. The shipping builds are slower than they need to be

The Rust compiler can be told "make builds quick" or "make the resulting
program quick" — it can't do both. During the recent redesign we picked
"make builds quick" so we could iterate fast. Every release we ship to
users is still built with that setting, so they get a version that's
running maybe 20-40% slower than it could.

> **Why this matters:** Flipping one config line in the build pipeline
> hands every user a free speed boost on the audio engine.
> Effort: ~15 minutes. Risk: zero.

### 6. Project files always start with 500 tracks

When you create a new project, the engine immediately builds out the
data for 500 mixer slots — even if you only ever use 10. They sit
there in memory and run through the engine on every audio block.

> **Why this matters:** Smaller projects pay the cost of pretending to
> be huge. Treating empty tracks as "not active yet" until you actually
> use them would let the engine spend its energy on real music.
> Effort: 1-2 days.

---

## What's already been fixed (recently)

The recent redesign sprint shipped a lot of perf work. Most of it is
real and works, but only inside the new mixer (which is off by default
— see #1 above). Here's the status:

| Improvement | Status |
|---|---|
| Only draw mixer strips you can see (virtualization) | ✅ Built, gated to new mixer |
| Scroll runs on the GPU, not the renderer | ✅ Built, gated to new mixer |
| Level meters draw directly to canvas — no React updates per frame | ✅ Built, gated to new mixer |
| Off-screen strips skip drawing entirely | ✅ Built, gated to new mixer |
| Smooth springy scroll feel | ✅ Built, gated to new mixer |
| Fader drag doesn't slam the audio engine every pixel | ✅ Built, but still triggers full re-fetch on release (see #3 above) |
| Audio device switch no longer pitches audio | ✅ Fixed in v0.159.1 |
| Spacebar→Stop preserves recording | ✅ Fixed in v0.158.9 |
| Plug-in state saves correctly with project | ✅ Fixed in v0.159.0 |

---

## What's still broken (not perf, but real)

Four known issues exist outside performance that are bigger blockers for
beta-readiness than any speed-up:

1. **Closing the window without saving discards your work** — no "Are
   you sure?" prompt. Data-loss risk.
2. **MIDI keyboards don't play instruments yet.** The wiring exists,
   but the audio engine never reads the events.
3. **MIDI doesn't reach plug-ins.** Software synths receive empty MIDI
   buffers, so they stay silent.
4. **Plug-ins without their own visual interface show no controls.**
   Older "utility" plug-ins are functionally inaccessible.

These four take roughly 10 engineering hours combined. Fixing them
turns the DAW from "demo-ready" into "beta-ready" regardless of any
perf work.

---

## What we'd do next, in order

If the goal is making the DAW feel snappier without breaking anything,
these are the changes in order of effort vs. impact:

| Order | Change | Effort | Effect |
|---|---|---|---|
| 1 | Turn on the new mixer by default | 15 min | Every user gets the work we already did |
| 2 | Flip the build pipeline to the optimised release profile | 15 min | Audio engine runs ~20-40% faster |
| 3 | Stop the audio thread from asking for memory mid-playback | ~6 hrs | Smoother under low-latency settings, no random glitches |
| 4 | One IPC call per fader move instead of 500 | ~6 hrs | Fader drags stay smooth on big projects |
| 5 | Skip silent tracks in the engine | ~1-2 days | "500 tracks supported" becomes honest, not aspirational |

After those five, we should measure with real profilers before deciding
what's next. There's a long tail of smaller optimisations — replacing
JSON between audio and display, smarter undo history, parallel audio
processing — but they only matter once these five are in.

---

## How to read this report

- **"Built, gated to new mixer"** means the code exists, works correctly,
  and ships with the app — but a feature flag hides it from most users.
- **"Audio thread"** is the part of the program that talks to your
  speakers. It has the strictest deadlines (milliseconds at a time) and
  is the most sensitive to performance issues.
- **"IPC round-trip"** is a tiny message between the audio side of the
  app and the visual side. They aren't free; doing 500 of them in a
  row stutters.
- **"Plug-in chain"** is the column of effects on a mixer track
  (compressor, EQ, reverb, etc.).
- **Cost estimates** are rough — they assume an experienced engineer
  working in a focused block, not interrupted, with the existing
  codebase already in their head.

---

*Last refreshed 2026-05-12 against branch `redesign-port-from-master`.
For the engineering-grade version with code references and exact line
numbers, see [the technical audit](https://github.com/Dishairano/hardwave-daw/blob/redesign-port-from-master/docs/perf-audit.md).*
