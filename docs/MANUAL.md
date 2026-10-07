# Hardwave DAW manual

This is the manual for the DAW as it is today. Everything in it is
something the program does now; nothing here is planned or coming.

## 1. Before you start

Hardwave DAW runs on Windows. The installer does not need administrator
rights: it installs for the person who runs it. Windows SmartScreen
warns about it because the installer is not signed yet, so the first
time you have to choose "More info" and then "Run anyway".

On first launch the setup wizard asks for an audio device and a MIDI
input. You can change both later in Options > Settings.

## 2. The window

- **Playlist** is the arrangement: tracks down the side, time across.
- **Channel rack** holds the pattern instruments and their steps.
- **Piano roll** edits the notes of one clip.
- **Mixer** holds the faders, the effect slots and the VCA groups.
- **Browser** shows the folders you have added, with a waveform for
  each sample.

F5 to F9 open them, and F12 closes everything that is open.

## 3. Sound out and in

Options > Settings > Audio picks the driver, the device, the sample
rate and the buffer size. A smaller buffer means less delay and more
load; if the audio crackles, raise it.

The Recording page shows the round trip the driver reports and lets you
add your own offset in milliseconds. What you record lands where you
played it, not where it arrived.

On an interface with more than two ins and outs, the Audio page picks
which pair you record from and which pair the mix goes out of.

## 4. Your first sound

The quickest way in is a starter song. File > New project offers four:
hardstyle and rawstyle at 150 BPM, frenchcore at 195, uptempo at 200.
Each has a kick from the built-in kick synth tuned for the style, an
offbeat bass in the drops, markers for intro, build, drop, break,
second drop and outro, empty tracks for a lead, a screech, atmos,
vocals and FX, and a limiter on the master. Press space and it plays.
Everything in it is there to be replaced.

To start from nothing instead:

1. Add a MIDI track: right-click in the track list and choose the kind.
2. Open the piano roll with F7 and draw a note.
3. Press space.

The built-in instrument plays right away. To use your own plug-in, drop
it into a slot in the mixer strip for that track: every VST3 and CLAP
the scan found is in the list. Options > Settings > Plug-ins is where
you add folders and re-scan.

## 5. Recording

Arm a track, press the record button, and play. Four switches on the
toolbar change how recording behaves:

- **Blend record** (Ctrl+B) merges what you play into the clip that is
  already there instead of stacking a new one.
- **Wait for input** (Ctrl+I) parks the transport until your first note.
- **Step editing** writes each key you press into the clip and moves on
  by the snap value.
- **Multilink** (Ctrl+J) keeps MIDI learn armed, so you can map a whole
  controller in one pass.

With the loop on, every pass is kept. The last one plays and the earlier
ones sit underneath muted, so trying another take never loses the one
before it.

## 6. Takes and comping

Right-click a track and choose "Spread the takes onto lanes" to see the
passes side by side. Click the take you want, set the loop range over
the part you want from it, then Edit > "Use the selected take over the
loop range". Every pass is cut at both edges, your choice plays and the
others are muted over that range. Nothing is deleted, so choosing a
different take over the same range is one more click.

## 7. Editing the arrangement

- Insert or delete time across the whole song from the Edit menu. Clips,
  automation, markers and tempo changes all move together.
- Mark the loop range as a section, then repeat that section with
  everything in it.
- Split, duplicate, slip, mute and warp a clip to the grid from the
  clip's own menu.
- One gesture is one undo, including a recording that landed on several
  tracks at once.

## 8. The piano roll

Draw, paint and erase notes. Snap follows the toolbar. Under the notes
is a strip that edits one property of each note: velocity, pan, fine
pitch in cents, or release. Pan and fine pitch move the built-in
instruments; a hosted plug-in needs note expression, which the host does
not send yet.

The strip beside it holds controller lanes: mod wheel, pitch bend,
sustain and any CC you add. What you draw there is saved inside the
clip, so it travels with the clip when you move it.

You can also take the groove off a played part, its timing and its
accents, and put it on a typed one.

## 9. MIDI effects

Right-click a MIDI track and choose "MIDI effects…". The chain sits
between the clips and the instrument, so the clip keeps the notes you
wrote and the chain decides what the synth hears:

- **Arpeggiator**: step, order, octaves and gate.
- **Chord**: one note becomes a shape.
- **Scale**: notes outside the scale move to the nearest one in it.
- **Transpose**: everything moves by a number of semitones.

Live playing goes through chord, scale and transpose. The arpeggiator
needs a clock of its own, so it works on what is written in a clip.

A MIDI track can also send its notes to other MIDI tracks, from the same
menu, so one part plays several instruments at once.

## 10. Mixing

Each track has a fader, pan, a width control and ten effect slots. Sends
go to any other track, pre or post fader. The VCA rail beside the faders
holds groups: a group's fader is added to each member's own, and nothing
is rerouted, so where the effects sit and what the sidechain keys off
stay as they were.

Automation records from the faders and knobs while you play: write,
touch or latch. Trim mode rides the automation that is already there
instead of replacing it. Send levels follow their automation too.

Macro knobs, under Tools, make one control that moves several parameters
across several plug-ins. Each link says the two values the parameter
travels between, so one knob can open one filter while it closes
another.

Tools > Modulation is the other half of that: a shape that keeps
running, wired to a plug-in's knob. Pick the knob, a shape, a rate and
a depth, and it moves for the whole song without a single automation
point. The knob's own value becomes the middle of the swing, so
switching a route on does not jump the sound.

A track whose plug-ins have done their work can be frozen from its
own menu: the DAW plays a render of it and gives the CPU back, while
the part, the plug-ins and the automation stay exactly where they are.
Unfreezing puts the live chain back with nothing lost.

## 11. Plug-ins

A plug-in is loaded in a throwaway process first, so one that crashes on
load cannot take the song with it. Once loaded it runs inside the DAW,
so a plug-in that crashes during playback still can.

Tools > Presets lists every preset you have saved, across every plug-in,
and loads one into the slot you pick. A VST3's own presets are listed
there too, once you pick a slot running it. A CLAP's are not: those come
through a factory the host does not read yet.

Turning a knob inside a plug-in's own window records automation like any
other control, as long as automation write is on.

## 12. Your keyboard, your language, your desk

The playlist and the piano roll are drawn on canvases, so they used to
be mouse-only. Both take focus now. In the playlist the arrows walk the
tracks and the clips on them, Ctrl with left or right nudges a clip by
the snap value, Enter opens a pattern and Delete removes a clip. In the
piano roll Tab walks the notes and Shift with Tab walks back. What the
keyboard lands on is announced, so a screen reader has something to
read.

Settings > Appearance picks the language. Anything not translated yet
stays in English rather than going blank.

Settings > MIDI also joins an Ableton Link session: the tempo and the
start and stop are then shared with everything else on the network that
speaks Link, and the row says how many other apps are in the session.

Settings > MIDI switches on a Mackie Control or HUI desk: eight faders
with mute, solo and arm, the transport keys, and bank left and right,
with the faders and mute lights sent back so a motorised desk lines up
with the mix. The scribble strips and the LED rings are not driven.

A MIDI track can also read MPE, from its own menu, so a controller can
bend one note of a chord without bending the rest. And a track can take
a tuning from a Scala file, which the built-in instruments follow; a
hosted plug-in keeps its own tuning.

## 13. Lining up two recordings

A kick recorded with a close mic and a room mic is the same hit twice,
a few milliseconds apart, and mixed together the gap eats the low end.
Right-click the track and choose "Line up with another track": the gap
is measured from the audio itself and taken out with the track's own
delay, so nothing on the timeline moves. If the two agree better with
one of them turned upside down, that is done as well and the message
says so. If they are not recordings of the same thing, nothing is
changed.

## 14. A plug-in in its own process

Right-click a plug-in in the mixer and switch "Own process" on, and it
is loaded into a process of its own the next time it starts. A crash
in it then takes that process, not the DAW: the slot goes quiet, the
song keeps playing, and a message says which plug-in stopped.

It costs one buffer of latency, reported so delay compensation lines
the track up with the rest, and a sandboxed plug-in uses the generic
parameter sheet rather than its own window. Leave it off for plug-ins
that behave, switch it on for the one that keeps falling over.

## 15. A phone as a remote (OSC)

Settings > MIDI has an OSC switch and a port, 9000 unless you change
it. Point TouchOSC or anything else that speaks OSC at this machine
and these addresses work:

```
/hardwave/play          /hardwave/stop        /hardwave/record
/hardwave/rewind        /hardwave/forward
/hardwave/tempo         bpm as a float
/hardwave/goto          beats from the start
/hardwave/bank/left     /hardwave/bank/right
/hardwave/master/volume 0 to 1
/hardwave/track/1/volume  0 to 1, strips count from 1
/hardwave/track/1/pan     -1 to 1
/hardwave/track/1/mute    /solo   /arm
```

Strips follow the same bank of eight as a control surface, so the bank
buttons move both. An address we do not know is ignored rather than
guessed at.

## 16. Scripts

Some edits are a loop, not a gesture: forty hats on the off-beat,
every track down three dB, a clip moved a bar. Tools > Scripts writes
them once and runs them whenever. A run is one undo step, however
much it does.

The language is Rhai, which reads like plain Rust. What a script can
call:

```
play()  stop()  seek(tick)
set_volume(track_id, db)   set_pan(track_id, -1..1)
set_muted(track_id, true)  set_master_volume(db)
add_note(clip_id, tick, pitch, velocity, length)
delete_note(clip_id, tick, pitch)
move_clip(clip_id, tick)   delete_clip(clip_id)
beats(n)  bars(n)          so you can write beats(2)
print(text)
```

Check reads the script and says what it would do without changing
anything. A command naming something that is not there is skipped and
counted, so the run says "12 of 16 applied" rather than stopping half
way. A script cannot reach the disk or the network, and one that
loops forever is stopped rather than left running.

## 17. A hummed line into notes

Right-click an audio clip and choose "Turn into notes". The pitch is
followed and written as a MIDI clip on a new track under the audio,
at the same place on the timeline, so the two line up and you can
hear them against each other.

One voice at a time. A chord is several pitches at once and this
follows the loudest, so it is for a sung line, a bassline or a lead,
not a mixdown. A clip with no pitch in it says so rather than
inventing notes.

## 18. Tuning a take

Right-click an audio clip and choose "Tune this take". Each note is
moved by its own amount, which is the point: a singer is sharp on one
word and flat on the next.

Strength decides how far towards the note it goes. Hard dance wants
all of it; a sung chorus usually does not. A key can be set, so a
note outside it is pulled to the nearest note of the key rather than
to the nearest semitone, and "leave alone within" keeps vibrato and a
human edge by ignoring anything already that close.

The take keeps its length and its place on the timeline. The tuned
audio is written as a new file and the clip is pointed at it, so the
take as it was sung is still there.

## 19. Painting something out

A cough in a vocal take, a chair creak under a verse, a click in a
bounce: in time they are mixed in with everything else, but on a
spectrogram they sit in their own patch. Right-click the clip and
choose "Paint something out", drag a box around the patch, and it is
rubbed out.

Strength decides how much goes. Part way is often better than all of
it, because a hole can be as noticeable as the noise was. The result
is written as a new file and the clip is pointed at it, so the
recording is kept.

## 20. The clip launcher

A timeline is for writing a song down. A grid of loops is for finding
one, and for playing live. Tools > Clip launcher opens it: a column
per track, a row per scene.

Click an empty cell to load a loop, click a full one to launch it.
Launching waits for the next boundary, which is a bar unless you
change it at the top, so whatever you press lands in time with
whatever is already going. A scene button launches a whole row at
once: tracks with a clip in that row start, tracks without one stop,
which is what makes a row a section. The square under a column stops
that track at the next boundary, and Stop all stops everything now,
because a stop button that waits is not a stop button.

Right-click a cell to empty it. A launched clip replaces that track's
timeline clips while it plays, and an export renders the song as
written rather than whatever was launched.

## 21. Opening an FL Studio project

File > Import FL Studio project reads a .flp. What comes across is
the arrangement: the tempo, a track per channel that plays something,
named as it was named in FL, and the notes of every pattern where the
playlist puts them.

What cannot come across is everything a plug-in holds. FL keeps a
plug-in's state in its own format, and nothing outside FL can mean
anything by it, so the message after an import says what was left
behind rather than letting you find out later. Sample paths point at
the machine the project came from, so they are listed rather than
loaded.

## 22. Scoring to picture

Tools > Video opens a film or a game capture in a corner of the
window. It follows the playhead: scrub and the picture scrubs, play
and it plays, so a hit can be written where the cut is rather than
where it sounded about right.

"Starts at bar" moves the film against the song. Its own sound is off
to begin with, because dialogue under a track being written is rarely
what anyone wants, and the button turns it on. The playhead drives
the video and never the other way round, so a frame arriving late
cannot drag the song with it.

## 23. Old 32-bit plug-ins

Plenty of free plug-ins from the 2000s were never rebuilt for 64-bit,
and hard dance is full of them. A 64-bit program cannot load a 32-bit
plug-in at all, so these run in a small helper that ships with the
DAW and speaks to it over a pipe.

Nothing to switch on: the architecture is read out of the plug-in's
own header, and one that does not match is sent to the helper. It
costs one buffer of latency, reported so delay compensation lines the
track up, and such a plug-in uses the generic parameter sheet rather
than its own window. A crash in it takes the helper, not the song.

## 24. Working on a song together

Tools > Work together opens a room and gives you a code. Read it out
to the other person; they type it in and you are both in the same
song. What crosses the network is the edits, not the audio, so you
each hear your own machine at full quality and a bad connection
costs a late edit rather than a dropout. The playhead is shared, so
"listen to this bit" means the same bar on both screens.

Opening a room is part of Hardwave Pro. Joining one is free: you
need an account so the other person can see who is in their song,
and nothing more. Both of you need the plug-ins the song uses; ours
are free inside this DAW, so that part takes care of itself.

The panel shows who is in the room, and the arrangement shows where
the other person is working: a dashed violet line with their name on
the ruler, wherever they last clicked. The red line is still the
playhead, and the two are never the same colour on purpose.

If you do not have the song yet, press "Get the song from them" and
it is sent over once. That replaces what is open on your side. The
audio files do not travel with it, so a clip whose sample you do not
have reads as missing, exactly as it does when a project is copied
between machines by hand.

## 25. Songs in Workspace

File > Save to Workspace saves the open song, collects the samples it
uses into a folder beside it, and puts the whole folder in your
Workspace under "Hardwave DAW". Saving the same song again only sends
what changed; Workspace keeps the earlier versions.

File > Open from Workspace lists the songs there. Opening one brings
the folder down to Documents/Hardwave/From Workspace and opens it, with
the samples beside it, so it plays as it did on the machine that saved
it. You sign in once, in any Hardwave plug-in; the DAW uses the same
sign-in.

## 26. Mixing against a record

Tools > Reference track loads a commercial track and plays it instead
of your mix, past the master chain and the master fader, so what you
hear is the record itself. "Match loudness" puts it at the loudness
your mix is measuring, because louder always sounds better and
comparing at two levels compares the levels rather than the mixes.

The reference follows the playhead, so moving in the song moves in the
record.

## 27. Exporting

File > Export renders the song to WAV or MP3, with the same plug-in
settings and sidechain routing as playback. You can render stems, one
file per track, and each stem keeps its own sidechain key.

Bounce a single track to audio from the track's menu when you want its
CPU back.

## 28. When something goes wrong

- **The audio stops**: the banner offers a retry. Check the device is
  still there in Settings > Audio.
- **It crackles**: Tools > Performance test measures how many tracks
  this PC can play at your buffer size, in about a minute, without
  touching your song. If multi-core audio is off it also measures
  with it on, and offers to switch it on. A larger buffer gives every
  block more time, at the cost of a little delay when you play live.
- **A plug-in window is blank**: the DAW writes a log next to your
  projects. Help > Report a bug attaches it.
- **A sample is missing**: the DAW offers to relink it, and it can find
  a file that moved or was renamed.
- **The project will not open**: a song saved by a newer version says so
  rather than opening wrong.

Help > Report a bug sends what you write, with the version and the log,
straight to us.
