# /// script
# requires-python = ">=3.10"
# dependencies = ["numpy", "scipy"]
# ///
"""Makes asides' sounds, crates/asides-app/sounds/*.wav, in code: a soft marimba in a small room.

Microphone on rises a fourth (A5 to D6) and off falls back; sent rises D6 F#6 A6 after a breath of
air; closing without sending falls F#5 to D5, muted; an error knocks twice on D4; a dictation
joining is one short F#6. Each is as loud as the macOS sound it replaced (the mic sounds a little
quieter, as they play most), starts at once and peaks well under full scale.

Usage: uv run scripts/make-sounds.py
"""

import pathlib

import numpy as np
from scipy.io import wavfile
from scipy.signal import butter, lfilter, sosfilt

ROOT = pathlib.Path(__file__).resolve().parent.parent
OUT = ROOT / "crates" / "asides-app" / "sounds"
SR = 48000


def note(name):
    semis = {"C": -9, "D": -7, "E": -5, "F": -4, "G": -2, "A": 0, "B": 2}[name[0]] + name.count("#")
    return 440.0 * 2 ** ((semis + 12 * (int(name[-1]) - 4)) / 12)


def place(parts):
    """Mixes (seconds, signal) pairs."""
    out = np.zeros(max(int(at * SR) + len(y) for at, y in parts))
    for at, y in parts:
        out[int(at * SR) : int(at * SR) + len(y)] += y
    return out


def bandpass(y, lo, hi):
    return sosfilt(butter(2, [lo, hi], "bandpass", fs=SR, output="sos"), y)


def fade_in(y, seconds):
    n = max(1, int(seconds * SR))
    y[:n] *= 0.5 - 0.5 * np.cos(np.pi * np.arange(n) / n)
    return y


def fade_out(y, n):
    y[-n:] *= np.cos(np.linspace(0, np.pi / 2, n)) ** 2
    return y


def wood(name, dur=0.32, ring=0.11, mute=1.0, contact=0.10):
    """A struck marimba bar: overtones near 4 and 10 times the note that die away first (`mute`
    turns them down), and the mallet's contact, a few milliseconds of noise."""
    f = note(name)
    t = np.arange(int(dur * SR)) / SR
    y = sum(
        amp * np.sin(2 * np.pi * f * ratio * t) * np.exp(-t / tau)
        for ratio, amp, tau in [(1, 1.0, ring), (3.99, 0.22 * mute, ring * 0.32), (9.2, 0.05 * mute, ring * 0.12)]
        if f * ratio < 16000
    )
    y = fade_out(fade_in(y, 0.0015), min(len(y) // 5, int(0.03 * SR)))
    n = int(0.004 * SR)
    burst = bandpass(noise.standard_normal(n), 1500, 6000) * np.exp(-np.arange(n) / (0.0008 * SR))
    y[:n] += contact * burst / np.abs(burst).max()
    return y


def air(dur, lo, hi, peak_at):
    """A breath of filtered noise that swells and fades."""
    t = np.arange(int(dur * SR)) / SR
    env = np.where(t < peak_at, (t / peak_at) ** 2, np.exp(-(t - peak_at) / (dur * 0.25)))
    return bandpass(noise.standard_normal(len(t)), lo, hi) * env


def room(y, mix=0.10, tau=0.045):
    """A small, dull room: a short noise tail under the dry sound, from 4 ms on (the onset stays clean)."""
    t = np.arange(int(tau * 6 * SR)) / SR
    ir = sosfilt(butter(2, 3500, "lowpass", fs=SR, output="sos"), noise.standard_normal(len(t))) * np.exp(-t / tau)
    ir[: int(0.004 * SR)] = 0
    wet = np.convolve(y, ir)
    return np.concatenate([y, np.zeros(len(wet) - len(y))]) + mix * wet * np.abs(y).max() / np.abs(wet).max()


def chime():
    return place([(0, wood("D6", ring=0.08)), (0.07, wood("F#6", ring=0.08)), (0.14, wood("A6", dur=0.7, ring=0.22))])


# Each sound and its loudness (the loudest 100 ms, LUFS): that of the macOS sound it replaced (Tink,
# Pop, Glass, Funk, Basso), less 1 to 6 dB for the frequent ones. Their noise (the mallet's contact,
# the room, the breath) comes from one generator, drawn in this order: the takes that were picked by
# ear. The chime without its breath was drawn among them, and isn't kept (no loudness).
SOUNDS = {
    "mic-on": (-24.7, lambda: place([(0, wood("A5", ring=0.07)), (0.055, wood("D6", ring=0.09))])),
    "mic-off": (-27.1, lambda: place([(0, wood("D6", ring=0.07)), (0.055, wood("A5", ring=0.09))])),
    "chime": (None, chime),
    "closed": (-23.4, lambda: place([(0, wood("F#5", ring=0.08, mute=0.5)), (0.09, wood("D5", ring=0.12, mute=0.5))])),
    "error": (-25.6, lambda: place([(0, wood("D4", ring=0.06, mute=0.3, contact=0.2)), (0.13, wood("D4", ring=0.08, mute=0.3, contact=0.2))])),
    "dictation": (-27.7, lambda: wood("F#6", dur=0.2, ring=0.05)),
    "sent": (-21.6, lambda: place([(0, 0.25 * air(0.22, 1500, 7000, 0.12)), (0.12, chime())])),
}


def loudness(y):
    """The loudest 100 ms, K-weighted as in ITU-R BS.1770 (its 48 kHz filters): about how long the
    ear sums a short sound, so a click and a ringing note compare as they sound."""
    k = lfilter([1.53512485958697, -2.69169618940638, 1.19839281085285], [1.0, -1.69065929318241, 0.73248077421585], y)
    k = lfilter([1.0, -2.0, 1.0], [1.0, -1.99004745483398, 0.99007225036621], k)
    power = np.convolve(k**2, np.ones(int(0.1 * SR)) / int(0.1 * SR))
    return -0.691 + 10 * np.log10(power.max())


def finish(y, target):
    """Starts the sound at once, ends it where it has died away (60 dB down), sets its loudness."""
    peak = np.abs(y).max()
    y = y[max(0, np.argmax(np.abs(y) > peak / 100) - int(0.0005 * SR)) :]
    y = y[: np.nonzero(np.abs(y) > peak / 1000)[0][-1] + 1]
    y = fade_out(y, min(len(y) // 4, int(0.08 * SR)))
    y[: int(0.0003 * SR)] *= np.linspace(0, 1, int(0.0003 * SR))
    y *= 10 ** ((target - loudness(y)) / 20)
    return y * min(1, 0.5 / np.abs(y).max())  # peaks under -6 dBFS


noise = np.random.default_rng(7)
dither = np.random.default_rng(0)
OUT.mkdir(exist_ok=True)
for name, (target, make) in SOUNDS.items():
    y = room(make())
    if target is None:
        continue
    y = finish(y, target)
    y = np.round((y + (dither.random(len(y)) - dither.random(len(y))) / 32768) * 32767).astype(np.int16)
    wavfile.write(OUT / f"{name}.wav", SR, y)
    print(f"{name}.wav: {len(y) / SR:.2f} s, {loudness(y / 32767):.1f} LUFS (100 ms), peak {20 * np.log10(np.abs(y).max() / 32767):.1f} dBFS")
