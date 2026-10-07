"""Split one song into drums, bass, vocals and the rest.

Run by the stems service, one song at a time:

    separate.py <input file> <output folder>

It prints "progress <0..1>" lines while it works and "done" at the
end, and writes drums.flac, bass.flac, other.flac and vocals.flac at
the song's own sample rate, so each part lines up sample for sample
with the clip it came from.

The model is Demucs (htdemucs, MIT). The service runs on a small
machine beside other work, so the song goes through in chunks of half
a minute with the model loaded once: memory stays at what one chunk
needs however long the song is. Neighbouring chunks overlap by two
seconds and are crossfaded, so there is no seam where they meet.
"""

import os
import subprocess
import sys

import numpy as np
import soundfile as sf
import torch
from demucs.apply import apply_model
from demucs.pretrained import get_model

CHUNK_SECONDS = 30
OVERLAP_SECONDS = 2
LONGEST_SECONDS = 15 * 60


def decode(path: str, sample_rate: int) -> np.ndarray:
    """Any file ffmpeg reads, as float32 stereo at the model's rate."""
    raw = subprocess.run(
        ["ffmpeg", "-v", "error", "-i", path, "-f", "f32le", "-ac", "2",
         "-ar", str(sample_rate), "-"],
        check=True, capture_output=True,
    ).stdout
    return np.frombuffer(raw, dtype=np.float32).reshape(-1, 2).T.copy()


def original_rate(path: str) -> int:
    """The sample rate the song came in at, or 0 when it cannot be read."""
    probe = subprocess.run(
        ["ffprobe", "-v", "error", "-select_streams", "a:0", "-show_entries",
         "stream=sample_rate", "-of", "csv=p=0", path],
        capture_output=True, text=True,
    )
    try:
        return int(probe.stdout.strip().split()[0])
    except (ValueError, IndexError):
        return 0


def back_to_rate(path: str, rate: int) -> None:
    """Resample one finished part to the song's own rate, in place."""
    temporary = path + ".tmp.flac"
    subprocess.run(
        ["ffmpeg", "-v", "error", "-y", "-i", path, "-ar", str(rate),
         "-c:a", "flac", "-sample_fmt", "s32", "-bits_per_raw_sample", "24", temporary],
        check=True,
    )
    os.replace(temporary, path)


def main(source: str, out_dir: str) -> None:
    torch.set_num_threads(int(os.environ.get("STEMS_THREADS", "2")))
    model = get_model("htdemucs")
    model.eval()
    rate = model.samplerate

    audio = decode(source, rate)
    total = audio.shape[1]
    if total == 0:
        sys.exit("the file has no audio in it")
    if total > LONGEST_SECONDS * rate:
        sys.exit(f"songs up to {LONGEST_SECONDS // 60} minutes can be separated")

    # Normalise on the whole song, the way Demucs does, so every chunk
    # is scaled the same and the levels match across the joins.
    mono = audio.mean(0)
    centre, spread = float(mono.mean()), float(mono.std()) or 1.0

    os.makedirs(out_dir, exist_ok=True)
    writers = {
        name: sf.SoundFile(os.path.join(out_dir, f"{name}.flac"), "w", rate, 2, subtype="PCM_24")
        for name in model.sources
    }
    chunk, overlap = CHUNK_SECONDS * rate, OVERLAP_SECONDS * rate
    fade_in = np.linspace(0.0, 1.0, overlap, dtype=np.float32)
    tails = {}
    start = 0
    with torch.no_grad():
        while start < total:
            end = min(total, start + chunk + overlap)
            piece = torch.from_numpy((audio[:, start:end] - centre) / spread)
            parts = apply_model(model, piece[None], split=True, overlap=0.25)[0]
            parts = (parts * spread + centre).numpy()
            last = end == total
            for i, name in enumerate(model.sources):
                part = parts[i]
                if name in tails:
                    tail = tails.pop(name)
                    n = min(tail.shape[1], part.shape[1])
                    part[:, :n] = tail[:, :n] * (1.0 - fade_in[:n]) + part[:, :n] * fade_in[:n]
                if last:
                    writers[name].write(part.T)
                else:
                    writers[name].write(part[:, :chunk].T)
                    tails[name] = part[:, chunk:]
            start += chunk
            print(f"progress {min(1.0, start / total):.3f}", flush=True)

    for w in writers.values():
        w.close()
    # The model works at 44.1 kHz. A song that came in at another rate
    # goes back to it, so the DAW can lay each part exactly where the
    # clip was, trimmed the same way.
    rate_in = original_rate(source)
    if rate_in and rate_in != rate:
        for name in model.sources:
            back_to_rate(os.path.join(out_dir, f"{name}.flac"), rate_in)
    print("done", flush=True)


if __name__ == "__main__":
    if len(sys.argv) != 3:
        sys.exit("usage: separate.py <input> <output folder>")
    main(sys.argv[1], sys.argv[2])
