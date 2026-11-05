# Native macOS build

The packaged app targets macOS 15 and Apple silicon (arm64). The package script
pins the Go helper's minimum OS version to 15.0; Cargo builds for the host
architecture.

The native target is a GPUI desktop application that starts the Go Collector
and DuckDB backend as a child process. The UI and backend exchange
Content-Length framed JSON-RPC over the child's standard input and output. The
backend does not start the viewer's HTTP API in this mode. OTLP HTTP and gRPC
receivers remain available to ingest telemetry.

## Build and launch

On macOS, install the Go and Rust toolchains and Xcode Command Line Tools, then
run:

```sh
./native/scripts/package-macos.sh
open "dist/native/OTel Desktop Viewer.app"
```

The script builds the backend with `EVALS=0`, builds the Rust application with
Cargo's locked dependency versions, and assembles an ad-hoc signed app at
`dist/native/OTel Desktop Viewer.app`. It also leaves the backend at
`dist/native/otel-desktop-backend` for integration tests. The bundle includes
both executables, the app icon, `Info.plist`, the project license, and licenses
for the bundled Atkinson font and Madoromi theme assets. Ad-hoc signing supports
local development; distribution requires the distributor's Apple signing and
notarization credentials.

The child backend defaults to OTLP gRPC on `localhost:4317`, OTLP HTTP on
`localhost:4318`, and its local DuckDB file in the user's application data
directory. Native builds use the public `EVALS=0` backend. When running native
beside the web/CLI target, choose unused OTLP ports and a separate database so
the two processes do not contend for receiver ports or the same store. These
environment variables override the defaults:

- `OTEL_DESKTOP_BACKEND`: backend executable path.
- `OTEL_DESKTOP_DB`: DuckDB database path.
- `OTEL_DESKTOP_GRPC_PORT`: OTLP gRPC receiver port.
- `OTEL_DESKTOP_HTTP_PORT`: OTLP HTTP receiver port.

## Implemented native workflows

The native UI provides Home service discovery with typed identities and paging,
service sorting, a directed service map with pan/zoom/fit and selectable
connections, connection-span evidence, and per-signal telemetry search.
Attribute discovery supports typed keys and values, name/count/frequency key
sorting, distinct-record coverage for the selected signal and owner, per-tab
25/50/100 paging, exact copy, scoped telemetry, and expand/collapse previews for
the visible key page. Traces, logs, and metrics have separate search results,
pagination, and detail views; metric views include charts and datapoint
selection. Import reports, signal exports, appearance settings, adjustable
panels, storage usage, ingestion rejection diagnostics, and configured OTLP
addresses with copyable SDK environment settings are also available.

The native views follow the web viewer's navigation, compact headers, signal
cards, typed property tables, and seasonal interface palettes. Home keeps the
service map in the center and selected-service telemetry and attributes in the
right pane. The map uses stable placement rather than force-directed motion.
Both side panes resize by dragging their dividers; widths persist across
launches. Below 1280 pixels, Home, traces, and metrics stack the main view and
Details beside the list, with a draggable horizontal divider. Logs keep their
table and inspector side by side. Detail sections retain separate scroll
positions and adjustable heights. Key coverage is reported for the selected
signal and owner. Native builds use the
public `EVALS=0` backend. The local app is ad-hoc signed and is not notarized;
distribution requires the distributor's Apple signing and notarization.

## Stdio and gRPC integration checks

Build the app bundle first; this also builds the helper needed by the Rust
backend integration target. To run the tests directly:

```sh
cargo fmt --manifest-path native/Cargo.toml -- --check
cargo check --locked --manifest-path native/Cargo.toml --tests --test native_ui
cargo test --locked --manifest-path native/Cargo.toml --bin otel-desktop-native
cargo test --locked --manifest-path native/Cargo.toml --test backend_integration
python3 native/scripts/smoke-stdio.py dist/native/otel-desktop-backend
OTEL_NATIVE_ARTIFACTS="$PWD/dist/native/evidence" cargo test --locked --manifest-path native/Cargo.toml --test native_ui
frame_dir="$(find dist/native/evidence -type d -name frames | sort | tail -n 1)"
swift native/scripts/encode-evidence.swift "$frame_dir" "${frame_dir%/frames}/native-ui.mp4" 10
```

The native UI target runs production views with the real stdio backend in the
headless Metal renderer; it does not launch or capture a desktop window. Its
automated interaction flow covers Home service/attribute discovery, map
connection selection and evidence, attribute expand/collapse and sorting,
trace/log/metric details, raw OTLP export, metric plots and datapoint selection,
search, imports, theme changes, panel resizing, and a 900-pixel viewport. It
writes screenshots and fixed-cadence frames under a timestamped directory in
`dist/native/evidence/`. See [native/evidence/README.md](evidence/README.md) for
provenance and encoding details.

The backend integration test uses isolated OTLP ports and a temporary database
to check fixture imports, search/paging, details, exact typed values, and
scoped exports. The Python smoke harness sends traces, logs, and metrics using
real OTLP gRPC protobuf clients, checks the initialize handshake and stored
signal counts, exports all three signals through JSON-RPC, restarts against the
same database, checks that the viewer HTTP port stays closed, and confirms
that stdin EOF shuts down the child. Set `--sender PATH` to use an already-built
gRPC sender instead of `go run`.
