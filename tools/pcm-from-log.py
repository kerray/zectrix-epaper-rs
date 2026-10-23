#!/usr/bin/env python3
"""Turn the `audio` app's serial PCM dump into a WAV file and report on it.

Usage: pcm-from-log.py LOG [OUT.wav]

The app prints `PCM-START <frames> <rate> <slot>`, lines `PCM <first frame> <hex>` with
32 little-endian i16 samples each, and `PCM-END <sum>` (wrapping u16 sum of all samples)
after each playback. The last complete dump in LOG is used. Lost lines are filled with
silence and reported.

The report helps tell recording faults from playback faults:
- dropouts: 10 ms blocks that are digital silence or a repeated value in the middle of
  sound, and sudden jumps between blocks;
- level: peak, RMS and DC offset;
- tone: average level per frequency band over the loud blocks (Goertzel), to judge how
  "muffled" the microphone path is.
Standard library only.
"""

import math
import re
import struct
import sys
import wave

START = re.compile(r"PCM-START (\d+) (\d+) (\d+)")
LINE = re.compile(r"PCM (\d+) ([0-9a-f]+)\s*$")
END = re.compile(r"PCM-END (\d+)")
BANDS = [150, 300, 600, 1000, 1500, 2000, 3000, 4000, 5000, 6000, 7000]


def parse(path):
    dumps = []
    cur = None
    with open(path, errors="replace") as f:
        for raw in f:
            m = START.search(raw)
            if m:
                cur = {"frames": int(m[1]), "rate": int(m[2]), "slot": int(m[3]), "lines": {}}
                continue
            if cur is None:
                continue
            m = LINE.search(raw)
            if m and len(m[2]) % 4 == 0:
                cur["lines"][int(m[1])] = bytes.fromhex(m[2])
                continue
            m = END.search(raw)
            if m:
                cur["sum"] = int(m[1])
                dumps.append(cur)
                cur = None
    if not dumps:
        sys.exit("no complete PCM dump (PCM-START … PCM-END) in the log")
    return dumps[-1]


def samples_of(dump):
    n = dump["frames"]
    out = [0] * n
    missing = 0
    for first in range(0, n, 32):
        data = dump["lines"].get(first)
        count = min(32, n - first)
        if data is None or len(data) != 2 * count:
            missing += 1
            continue
        out[first : first + count] = struct.unpack(f"<{count}h", data)
    return out, missing


def goertzel_db(block, rate, freq):
    w = 2 * math.pi * freq / rate
    c = 2 * math.cos(w)
    s1 = s2 = 0.0
    for x in block:
        s1, s2 = x + c * s1 - s2, s1
    p = s1 * s1 + s2 * s2 - c * s1 * s2
    return 10 * math.log10(p / len(block) ** 2 + 1e-12)


def main():
    if len(sys.argv) not in (2, 3):
        sys.exit(__doc__)
    dump = parse(sys.argv[1])
    out_path = sys.argv[2] if len(sys.argv) == 3 else "recording.wav"
    s, missing = samples_of(dump)
    rate = dump["rate"]
    total = sum(s) & 0xFFFF
    print(f"{len(s)} samples, {len(s) / rate:.2f} s at {rate} Hz, slot {dump['slot']}")
    print(f"lines missing: {missing}; sum {'OK' if total == dump['sum'] else 'MISMATCH'}")

    with wave.open(out_path, "wb") as w:
        w.setnchannels(1)
        w.setsampwidth(2)
        w.setframerate(rate)
        w.writeframes(struct.pack(f"<{len(s)}h", *s))
    print(f"wrote {out_path}")

    if not s:
        return
    peak = max(abs(x) for x in s)
    dc = sum(s) / len(s)
    rms = math.sqrt(sum((x - dc) ** 2 for x in s) / len(s))
    clipped = sum(1 for x in s if abs(x) >= 32700)
    print(f"peak {peak} ({20 * math.log10(peak / 32768 + 1e-12):.1f} dBFS), "
          f"RMS {rms:.0f} ({20 * math.log10(rms / 32768 + 1e-12):.1f} dBFS), "
          f"DC {dc:.0f}, clipped samples {clipped}")

    # 10 ms blocks: level envelope, dropouts, jumps.
    bl = rate // 100
    blocks = [s[i : i + bl] for i in range(0, len(s) - bl + 1, bl)]
    levels = [math.sqrt(sum((x - dc) ** 2 for x in b) / bl) for b in blocks]
    loud = sorted(levels)[len(levels) * 3 // 4] if levels else 0
    drop = []
    for i, b in enumerate(blocks):
        flat = max(b) - min(b) <= 2
        around = [levels[j] for j in (i - 2, i - 1, i + 1, i + 2) if 0 <= j < len(levels)]
        if flat and around and min(around) > max(4 * levels[i], 200):
            drop.append(i)
    print(f"flat 10 ms blocks inside sound (dropouts): {len(drop)}"
          + (f" at {', '.join(f'{i / 100:.2f}s' for i in drop[:20])}" if drop else ""))
    jumps = []
    for i in range(1, len(s)):
        d = abs(s[i] - s[i - 1])
        if d > 8 * (rms + 50):
            jumps.append(i)
    print(f"sample-to-sample jumps > 8x RMS (clicks / splices): {len(jumps)}"
          + (f" at {', '.join(f'{i / rate:.3f}s' for i in jumps[:20])}" if jumps else ""))

    # Envelope, 100 ms per character.
    chars = " .:-=+*#%@"
    env = []
    for i in range(0, len(levels), 10):
        lv = max(levels[i : i + 10])
        env.append(chars[min(9, int(9 * lv / (max(levels) + 1e-9)))])
    print("envelope (100 ms/char): |" + "".join(env) + "|")

    # Tone over the loud blocks (top quarter), 32 ms windows.
    win = rate * 32 // 1000
    picks = [i for i, lv in enumerate(levels) if lv >= loud and lv > 100]
    if picks:
        acc = {f: 0.0 for f in BANDS if f < rate / 2}
        used = 0
        for i in picks[:200]:
            start = i * bl
            block = s[start : start + win]
            if len(block) < win:
                continue
            block = [(x - dc) * (0.5 - 0.5 * math.cos(2 * math.pi * k / (win - 1)))
                     for k, x in enumerate(block)]
            for f in acc:
                acc[f] += goertzel_db(block, rate, f)
            used += 1
        if used:
            ref = max(acc.values()) / used
            print(f"band levels over {used} loud windows (dB relative to the strongest):")
            for f, v in acc.items():
                print(f"  {f:5d} Hz {v / used - ref:6.1f} dB")


if __name__ == "__main__":
    main()
