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

## 4. Your first sound

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

## 11. Plug-ins

A plug-in is loaded in a throwaway process first, so one that crashes on
load cannot take the song with it. Once loaded it runs inside the DAW,
so a plug-in that crashes during playback still can.

Tools > Presets lists every preset you have saved, across every plug-in,
and loads one into the slot you pick. Presets that ship inside a plug-in
are not listed: that needs the program list a VST3 publishes, which the
host does not read yet.

## 12. Exporting

File > Export renders the song to WAV or MP3, with the same plug-in
settings and sidechain routing as playback. You can render stems, one
file per track, and each stem keeps its own sidechain key.

Bounce a single track to audio from the track's menu when you want its
CPU back.

## 13. When something goes wrong

- **The audio stops**: the banner offers a retry. Check the device is
  still there in Settings > Audio.
- **A plug-in window is blank**: the DAW writes a log next to your
  projects. Help > Report a bug attaches it.
- **A sample is missing**: the DAW offers to relink it, and it can find
  a file that moved or was renamed.
- **The project will not open**: a song saved by a newer version says so
  rather than opening wrong.

Help > Report a bug sends what you write, with the version and the log,
straight to us.
