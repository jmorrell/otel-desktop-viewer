# Native UI render evidence

The PNGs and MP4 produced by `native/tests/native_ui.rs` are rendered by GPUI's
headless Metal renderer from the production native views and real Collector
responses. They show native component rendering, not a recording of a window on
the macOS desktop. Evidence files are kept under the ignored
`dist/native/evidence/` directory and are not committed by default.

The UI test writes fixed-size animation frames as
`dist/native/evidence/run-<timestamp>-<pid>/frames/frame-00000.png` at a 10 fps
capture cadence. Encode a frame sequence with the Apple AVFoundation encoder:

```sh
swift native/scripts/encode-evidence.swift \
  dist/native/evidence/run-<timestamp>-<pid>/frames \
  dist/native/evidence/run-<timestamp>-<pid>/native-ui.mp4 10
```

Frames must be sequentially named `frame-*.png` and share the same dimensions.
The resulting MP4 contains a description metadata field identifying it as an
offscreen Metal-rendered sequence. The encoder does not synthesize UI states;
all content comes from the supplied production-render frames.

For a quick visual comparison at 1440×900 and 900×760, set
`OTEL_NATIVE_VISUAL_SMOKE=1` when running the UI target. This renders all four
public pages against the real backend and saves screenshots, but it skips the
full interaction walkthrough and must not be reported as a full UI-test pass.

After encoding a recording, inspect decoded MP4 frames from the beginning,
middle, and end to verify orientation as well as the source PNG screenshots.
