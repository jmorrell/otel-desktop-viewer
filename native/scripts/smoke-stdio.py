#!/usr/bin/env python3
"""Exercise the native stdio backend with real OTLP protobuf gRPC traffic."""

from __future__ import annotations

import argparse
import json
import os
import select
import socket
import subprocess
import sys
import tempfile
import time
from pathlib import Path


ROOT = Path(__file__).resolve().parents[2]


def free_port() -> int:
    with socket.socket() as sock:
        sock.bind(("127.0.0.1", 0))
        return sock.getsockname()[1]


class Backend:
    def __init__(self, command: list[str], db: Path, grpc_port: int, http_port: int, viewer_port: int):
        self.process = subprocess.Popen(
            command
            + [
                "--transport=stdio",
                "--host=127.0.0.1",
                f"--grpc={grpc_port}",
                f"--http={http_port}",
                f"--browser-port={viewer_port}",
                f"--db={db}",
                "--open-browser=false",
            ],
            cwd=ROOT,
            stdin=subprocess.PIPE,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
        )
        self.next_id = 1

    def rpc(self, method: str, params: object = None, timeout: float = 15, fragmented: bool = False) -> object:
        assert self.process.stdin and self.process.stdout
        request_id = self.next_id
        self.next_id += 1
        payload = json.dumps(
            {"jsonrpc": "2.0", "id": request_id, "method": method, "params": params},
            separators=(",", ":"),
        ).encode()
        header = f"Content-Length: {len(payload)}\r\n\r\n".encode()
        if fragmented:
            self.process.stdin.write(header[:7])
            self.process.stdin.flush()
            time.sleep(0.01)
            self.process.stdin.write(header[7:] + payload[: max(1, len(payload) // 2)])
            self.process.stdin.flush()
            time.sleep(0.01)
            self.process.stdin.write(payload[max(1, len(payload) // 2) :])
        else:
            self.process.stdin.write(header + payload)
        self.process.stdin.flush()

        deadline = time.monotonic() + timeout
        header = bytearray()
        while not header.endswith(b"\r\n\r\n"):
            remaining = deadline - time.monotonic()
            if remaining <= 0 or not select.select([self.process.stdout], [], [], remaining)[0]:
                raise TimeoutError(f"timed out waiting for {method} response")
            byte = os.read(self.process.stdout.fileno(), 1)
            if not byte:
                stderr = self.process.stderr.read().decode(errors="replace") if self.process.stderr else ""
                raise RuntimeError(f"backend exited before replying to {method}: {stderr}")
            header.extend(byte)
            if len(header) > 8192:
                raise RuntimeError("stdio response header exceeded 8 KiB")
        length = next(
            int(line.split(b":", 1)[1])
            for line in header.split(b"\r\n")
            if line.lower().startswith(b"content-length:")
        )
        body = bytearray()
        while len(body) < length:
            remaining = deadline - time.monotonic()
            if remaining <= 0 or not select.select([self.process.stdout], [], [], remaining)[0]:
                raise TimeoutError(f"timed out reading {method} response body")
            chunk = os.read(self.process.stdout.fileno(), length - len(body))
            if not chunk:
                raise RuntimeError("backend closed stdout in the middle of a JSON-RPC frame")
            body.extend(chunk)
        response = json.loads(body)
        if response.get("id") != request_id:
            raise AssertionError(f"response id mismatch: wanted {request_id}, received {response.get('id')}")
        if "error" in response:
            raise RuntimeError(f"{method} returned JSON-RPC error: {response['error']}")
        return response.get("result")

    def close(self) -> None:
        if self.process.stdin:
            self.process.stdin.close()
        try:
            code = self.process.wait(timeout=15)
        except subprocess.TimeoutExpired:
            self.process.kill()
            raise RuntimeError("backend did not exit after stdin EOF")
        stderr = self.process.stderr.read().decode(errors="replace") if self.process.stderr else ""
        if code:
            raise RuntimeError(f"backend exited with status {code}: {stderr}")


def start(command: list[str], db: Path) -> tuple[Backend, dict[str, int]]:
    grpc_port, http_port, viewer_port = free_port(), free_port(), free_port()
    backend = Backend(command, db, grpc_port, http_port, viewer_port)
    ready = backend.rpc("native.initialize", fragmented=True)
    assert ready["version"] == 1, ready
    assert {"export", "cancellation"}.issubset(ready["capabilities"]), ready
    assert ready["receiverPorts"] == {"grpc": grpc_port, "http": http_port}, ready
    with socket.socket() as sock:
        sock.settimeout(0.2)
        assert sock.connect_ex(("127.0.0.1", viewer_port)) != 0, "stdio mode opened the viewer HTTP port"
    return backend, ready["receiverPorts"]


def assert_startup_failure(command: list[str], db: Path) -> None:
    with socket.socket() as occupied:
        occupied.bind(("127.0.0.1", 0))
        occupied.listen()
        blocked_port = occupied.getsockname()[1]
        process = subprocess.Popen(
            command
            + [
                "--transport=stdio",
                "--host=127.0.0.1",
                f"--grpc={blocked_port}",
                f"--http={free_port()}",
                f"--browser-port={free_port()}",
                f"--db={db}",
                "--open-browser=false",
            ],
            cwd=ROOT,
            stdin=subprocess.PIPE,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
        )
        try:
            code = process.wait(timeout=15)
        except subprocess.TimeoutExpired:
            process.kill()
            raise AssertionError("backend did not report a busy OTLP receiver port")
        stdout = process.stdout.read() if process.stdout else b""
        stderr = process.stderr.read().decode(errors="replace") if process.stderr else ""
        assert code != 0, "backend unexpectedly started with its gRPC receiver port occupied"
        assert not stdout, f"backend wrote non-protocol output to stdout during startup failure: {stdout!r}"
        assert "address already in use" in stderr.lower() or "bind" in stderr.lower(), stderr


def stats(backend: Backend) -> dict:
    result = backend.rpc("getStats")
    if isinstance(result, str):
        result = json.loads(result)
    if not isinstance(result, dict):
        raise AssertionError(f"getStats returned an unexpected result: {result!r}")
    return result


def query_rows(backend: Backend, statement: str) -> list[list[object]]:
    result = backend.rpc("query", [statement, 10])
    if not isinstance(result, dict) or not isinstance(result.get("rows"), list):
        raise AssertionError(f"query returned an unexpected result: {result!r}")
    return result["rows"]


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("backend", help="path to otel-desktop-backend")
    parser.add_argument("--sender", help="path to a prebuilt native/tools/otlp-send binary")
    args = parser.parse_args()
    backend_command = [str(Path(args.backend).resolve())]
    sender_command = [str(Path(args.sender).resolve())] if args.sender else ["go", "run", "./native/tools/otlp-send"]

    with tempfile.TemporaryDirectory(prefix="otel-native-smoke-") as temp_dir:
        db = Path(temp_dir) / "smoke.duckdb"
        first, ports = start(backend_command, db)
        try:
            subprocess.run(
                sender_command + [f"--endpoint=127.0.0.1:{ports['grpc']}", "--service=native-smoke-grpc"],
                cwd=ROOT,
                check=True,
                timeout=45,
            )
            # The Collector batches telemetry with a one second timeout; wait
            # for its queued writes before querying the DuckDB store.
            time.sleep(1.5)
            current = stats(first)
            spans = current["traces"]["spanCount"]
            assert spans >= 1, f"expected an ingested span, got {spans}"
            logs = current["logs"]["logCount"]
            assert logs >= 1, f"expected an ingested log, got {logs}"
            metrics = current["metrics"]["dataPointCount"]
            assert metrics >= 1, f"expected an ingested metric, got {metrics}"
            trace_query = first.rpc("query", ["SELECT trace_id FROM spans", 10])
            assert "10000000000040008000000000000001" in json.dumps(trace_query).lower().replace("-", ""), trace_query

            queries = {"traces": None, "logs": None, "metrics": None}
            services = first.rpc("discoverTelemetry", [{
                "kind": "services", "startTime": None, "endTime": None,
                "term": "native-smoke-grpc", "limit": 100, "queries": queries,
            }])
            service_rows = [row[0] for row in services["rows"]]
            service = next(row for row in service_rows if row["serviceName"] == "native-smoke-grpc")
            identity = service["serviceIdentity"]
            keys = first.rpc("discoverTelemetry", [{
                "kind": "keys", "signal": "traces", "ownerType": "resource",
                "serviceIdentity": identity, "startTime": None, "endTime": None,
                "term": "native.smoke", "limit": 25, "queries": queries,
            }])
            assert any(row[0]["key"] == "native.smoke.transport" for row in keys["rows"]), keys
            transport_key = next(row[0] for row in keys["rows"] if row[0]["key"] == "native.smoke.transport")
            assert transport_key["count"] == 1 and transport_key["denominator"] == 1, transport_key
            assert transport_key["relativeFrequency"] == 1, transport_key
            sorted_keys = first.rpc("discoverTelemetry", [{
                "kind": "keys", "signal": "traces", "ownerType": "resource",
                "serviceIdentity": identity, "startTime": None, "endTime": None,
                "term": "", "limit": 1, "offset": 0, "sortBy": "name",
                "sortDirection": "asc", "queries": queries,
            }])
            assert sorted_keys["rows"][0][0]["key"] == "native.smoke.transport", sorted_keys
            assert sorted_keys["truncated"] is True, sorted_keys
            next_key = first.rpc("discoverTelemetry", [{
                "kind": "keys", "signal": "traces", "ownerType": "resource",
                "serviceIdentity": identity, "startTime": None, "endTime": None,
                "term": "", "limit": 1, "offset": 1, "sortBy": "name",
                "sortDirection": "asc", "queries": queries,
            }])
            assert next_key["rows"][0][0]["key"] == "service.name", next_key
            reverse_keys = first.rpc("discoverTelemetry", [{
                "kind": "keys", "signal": "traces", "ownerType": "resource",
                "serviceIdentity": identity, "startTime": None, "endTime": None,
                "term": "", "limit": 1, "offset": 0, "sortBy": "name",
                "sortDirection": "desc", "queries": queries,
            }])
            assert reverse_keys["rows"][0][0]["key"] == "service.name", reverse_keys
            count_keys = first.rpc("discoverTelemetry", [{
                "kind": "keys", "signal": "traces", "ownerType": "resource",
                "serviceIdentity": identity, "startTime": None, "endTime": None,
                "term": "", "limit": 1, "offset": 0, "sortBy": "count",
                "sortDirection": "desc", "queries": queries,
            }])
            assert count_keys["rows"][0][0]["count"] >= 1, count_keys
            frequency_keys = first.rpc("discoverTelemetry", [{
                "kind": "keys", "signal": "traces", "ownerType": "resource",
                "serviceIdentity": identity, "startTime": None, "endTime": None,
                "term": "", "limit": 1, "offset": 0, "sortBy": "frequency",
                "sortDirection": "desc", "queries": queries,
            }])
            assert frequency_keys["rows"][0][0]["relativeFrequency"] > 0, frequency_keys
            values = first.rpc("discoverTelemetry", [{
                "kind": "values", "signal": "traces", "ownerType": "resource",
                "serviceIdentity": identity, "key": "native.smoke.transport",
                "startTime": None, "endTime": None, "term": "", "limit": 25,
                "sortBy": "frequency", "sortDirection": "desc", "queries": queries,
            }])
            assert values["rows"][0][0]["value"] == {"kind": "string", "value": "grpc"}, values

            service_filter = {
                "id": "native-smoke-service",
                "type": "condition",
                "query": {
                    "field": {"name": "serviceIdentity", "type": "string", "searchScope": "field"},
                    "fieldOperator": "=", "value": identity,
                },
            }
            scoped_traces = first.rpc("searchTraceSummaries", {
                "startTime": None, "endTime": None, "query": service_filter, "limit": 25,
            })
            assert len(scoped_traces) == 1 and scoped_traces[0]["traceID"].replace("-", "").lower() == "10000000000040008000000000000001", scoped_traces

            exported = first.rpc(
                "native.export",
                {"signal": "traces", "reference": "10000000000040008000000000000001"},
            )
            assert "native-smoke-grpc-span" in exported["content"], exported
            assert exported["filename"].startswith("trace-"), exported

            log_rows = query_rows(first, "SELECT id::VARCHAR FROM logs")
            assert log_rows, "query did not return the OTLP gRPC log"
            log_export = first.rpc("native.export", {"signal": "logs", "reference": log_rows[0][0]})
            assert "native-smoke-grpc-log" in log_export["content"], log_export
            assert log_export["filename"].startswith("log-"), log_export

            metric_rows = query_rows(first, "SELECT id::VARCHAR FROM metrics")
            assert metric_rows, "query did not return the OTLP gRPC metric"
            metric_export = first.rpc("native.export", {"signal": "metrics", "reference": metric_rows[0][0]})
            assert "native.smoke.grpc.value" in metric_export["content"], metric_export
            assert metric_export["filename"].startswith("metric-"), metric_export
        finally:
            first.close()

        second, _ = start(backend_command, db)
        try:
            spans_after_restart = stats(second)["traces"]["spanCount"]
            assert spans_after_restart >= 1, "persistent DuckDB data did not survive backend restart"
        finally:
            second.close()

        assert_startup_failure(backend_command, Path(temp_dir) / "blocked.duckdb")

    print("stdio smoke passed: fragmented initialize, real OTLP gRPC ingestion, typed Home service/key/value discovery and key paging/sorting, identity-scoped summaries, all-signal export, restart, busy-port failure, no viewer HTTP port, and EOF shutdown")
    return 0


if __name__ == "__main__":
    try:
        raise SystemExit(main())
    except Exception as exc:
        print(f"stdio smoke failed: {exc}", file=sys.stderr)
        raise
