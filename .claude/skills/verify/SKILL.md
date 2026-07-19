---
name: verify
description: Build and drive the lapsify CLI end to end to verify changes at its real surface.
---

# Verifying lapsify

Single-crate Rust CLI. Requires ffmpeg on PATH for video output.

## Build & run

```bash
cargo build --release          # binary at target/release/lapsify
```

## Generate test frames (no camera needed)

```bash
mkdir -p /tmp/lv/frames
# uniform mid-gray frames: any output brightness change comes from lapsify
for i in 0 1 2 3 4; do
  ffmpeg -y -loglevel error -f lavfi -i "color=c=0x808080:size=320x240" \
    -frames:v 1 /tmp/lv/frames/f_0$i.png
done
# or moving test pattern: -f lavfi -i "testsrc=size=320x240:rate=1"
```

## Flows worth driving

```bash
# image mode + measurable exposure ramp (-1EV..+1EV over the clip)
target/release/lapsify -i /tmp/lv/frames -o /tmp/lv/imgs -e "-1,1" -f jpg
# measure output luma; for gray 0x80 input expect ~91 at -1EV, ~173 at +1EV
ffmpeg -i /tmp/lv/imgs/f_00_processed.jpg \
  -vf "signalstats,metadata=print:key=lavfi.signalstats.YAVG" -f null - 2>&1 | grep -o "YAVG=[0-9.]*"

# video via project file (keyframed curves + crop), then inspect
target/release/lapsify --project project.json
ffprobe -v error -count_frames -show_entries \
  stream=codec_name,width,height,nb_read_frames -of csv=p=0 out/timelapse.mp4

# NDJSON progress purity: stdout must be only JSON lines
target/release/lapsify --project project.json --progress json 2>/dev/null
```

## Gotchas

- Human-readable output goes to stderr; piping stdout through `head` mid-run
  kills the process (SIGPIPE) and leaves partial output — capture fully.
- Video needs even dimensions; the encoder handles it, but ffprobe the result
  rather than assuming.
- Validation (crop bounds, ranges, codec/container rules) happens before any
  processing — error probes return fast with exit 1.
