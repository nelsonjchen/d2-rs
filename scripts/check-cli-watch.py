#!/usr/bin/env python3
# SPDX-License-Identifier: MPL-2.0
"""Smoke-test the native CLI watch page, WebSocket payload, and import reload."""

from __future__ import annotations

import base64
import json
import os
import queue
import signal
import socket
import subprocess
import tempfile
import threading
import time
import urllib.request
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]
BINARY = ROOT / "target" / "debug" / "d2-rs"


def read_exact(connection: socket.socket, length: int) -> bytes:
    result = bytearray()
    while len(result) < length:
        chunk = connection.recv(length - len(result))
        if not chunk:
            raise RuntimeError("preview server closed the WebSocket")
        result.extend(chunk)
    return bytes(result)


def read_text_frame(connection: socket.socket) -> str:
    first, second = read_exact(connection, 2)
    opcode = first & 0x0F
    length = second & 0x7F
    if length == 126:
        length = int.from_bytes(read_exact(connection, 2), "big")
    elif length == 127:
        length = int.from_bytes(read_exact(connection, 8), "big")
    if second & 0x80:
        mask = read_exact(connection, 4)
        payload = bytearray(read_exact(connection, length))
        for index in range(length):
            payload[index] ^= mask[index % 4]
    else:
        payload = bytearray(read_exact(connection, length))
    if opcode == 0x9:
        return read_text_frame(connection)
    if opcode != 0x1:
        raise RuntimeError(f"expected a text WebSocket frame, got opcode {opcode}")
    return payload.decode("utf-8")


def connect_websocket(host: str, port: int) -> socket.socket:
    connection = socket.create_connection((host, port), timeout=10)
    key = base64.b64encode(os.urandom(16)).decode("ascii")
    request = (
        f"GET /watch HTTP/1.1\r\nHost: {host}:{port}\r\n"
        "Upgrade: websocket\r\nConnection: Upgrade\r\n"
        f"Sec-WebSocket-Key: {key}\r\nSec-WebSocket-Version: 13\r\n\r\n"
    )
    connection.sendall(request.encode("ascii"))
    response = bytearray()
    while not response.endswith(b"\r\n\r\n"):
        response.extend(read_exact(connection, 1))
    if not response.startswith(b"HTTP/1.1 101 Switching Protocols"):
        raise RuntimeError(f"unexpected WebSocket response: {response!r}")
    return connection


def wait_for_address(process: subprocess.Popen[str], lines: queue.Queue[str]) -> tuple[str, int]:
    deadline = time.monotonic() + 30
    while time.monotonic() < deadline:
        if process.poll() is not None:
            raise RuntimeError(f"watch process exited early with {process.returncode}")
        try:
            line = lines.get(timeout=0.25)
        except queue.Empty:
            continue
        if line.startswith("listening on http://"):
            address = line.removeprefix("listening on http://").strip()
            host, port = address.rsplit(":", 1)
            return host.strip("[]"), int(port)
    raise TimeoutError("watch server did not report its listening address")


def main() -> None:
    if not BINARY.is_file():
        raise SystemExit(f"build the CLI first: cargo build -p d2-rs --bin d2-rs")
    with tempfile.TemporaryDirectory(prefix="d2-rs-watch-") as temporary:
        directory = Path(temporary)
        source = directory / "main.d2"
        imported = directory / "import_lib.d2"
        output_path = directory / "rendered.png"
        source.write_text(
            "...@import_lib\nlayers: { preview: { layer-node: in-layer } }\n",
            encoding="utf-8",
        )
        imported.write_text("imported: before-watch\n", encoding="utf-8")
        environment = os.environ.copy()
        environment["D2_WATCH"] = "0"
        environment["BROWSER"] = "0"
        process = subprocess.Popen(
            [
                str(BINARY),
                "--watch",
                "--host=127.0.0.1",
                "--port=0",
                "--browser=0",
                "--stdout-format=png",
                str(source),
                str(output_path),
            ],
            cwd=ROOT,
            env=environment,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            text=True,
        )
        assert process.stderr is not None
        lines: queue.Queue[str] = queue.Queue()
        reader = threading.Thread(
            target=lambda: [lines.put(line.rstrip()) for line in process.stderr],
            daemon=True,
        )
        reader.start()
        websocket: socket.socket | None = None
        try:
            host, port = wait_for_address(process, lines)
            root_url = f"http://{host}:{port}/"
            with urllib.request.urlopen(root_url, timeout=10) as response:
                page = response.read().decode("utf-8")
            if "d2-svg-container" not in page:
                raise RuntimeError("preview root page omitted the SVG container")
            with urllib.request.urlopen(f"{root_url}static/watch.js", timeout=10) as response:
                if b"new WebSocket" not in response.read():
                    raise RuntimeError("preview static client was not served")
            with urllib.request.urlopen(f"{root_url}static/watch.css", timeout=10) as response:
                if b"#d2-err" not in response.read():
                    raise RuntimeError("preview stylesheet was not served")
            with urllib.request.urlopen(f"{root_url}static/favicon.ico", timeout=10) as response:
                if not response.read().startswith(b"\x00\x00\x01\x00"):
                    raise RuntimeError("preview favicon was not served")

            websocket = connect_websocket(host, port)
            first = json.loads(read_text_frame(websocket))
            if first["err"] or "before-watch" not in first["svg"]:
                raise RuntimeError(f"initial preview compile failed: {first['err']}")
            first_png = output_path.read_bytes()
            if not first_png.startswith(b"\x89PNG\r\n\x1a\n"):
                raise RuntimeError("watch did not write the requested PNG output")

            imported.write_text("imported: after-watch\n", encoding="utf-8")
            deadline = time.monotonic() + 20
            while time.monotonic() < deadline:
                message = json.loads(read_text_frame(websocket))
                if "after-watch" in message["svg"]:
                    if message["err"]:
                        raise RuntimeError(message["err"])
                    updated_png = output_path.read_bytes()
                    if updated_png == first_png:
                        raise RuntimeError("watch did not refresh the requested PNG output")
                    break
            else:
                raise TimeoutError("preview did not reload after an imported file changed")

            imported.write_text("imported: {\n", encoding="utf-8")
            deadline = time.monotonic() + 20
            while time.monotonic() < deadline:
                message = json.loads(read_text_frame(websocket))
                if message["err"]:
                    break
            else:
                raise TimeoutError("preview did not report an imported-source error")

            imported.write_text("imported: recovered-watch\n", encoding="utf-8")
            deadline = time.monotonic() + 20
            while time.monotonic() < deadline:
                message = json.loads(read_text_frame(websocket))
                if "recovered-watch" in message["svg"]:
                    if message["err"]:
                        raise RuntimeError(message["err"])
                    break
            else:
                raise TimeoutError("preview did not recover after fixing imported source")

            with urllib.request.urlopen(
                f"http://{host}:{port}/layers/preview/index.svg", timeout=10
            ) as response:
                if "d2-svg-container" not in response.read().decode("utf-8"):
                    raise RuntimeError("board preview route did not return the page")
            deadline = time.monotonic() + 20
            while time.monotonic() < deadline:
                message = json.loads(read_text_frame(websocket))
                if message["err"]:
                    raise RuntimeError(message["err"])
                if "in-layer" in message["svg"]:
                    break
            else:
                raise TimeoutError("preview route did not select the requested board")
            print("CLI watch preview, board routing, dependency reload, errors, and recovery passed.")
        finally:
            if websocket is not None:
                websocket.close()
            process.send_signal(signal.SIGINT)
            try:
                process.wait(timeout=10)
            except subprocess.TimeoutExpired:
                process.kill()
                process.wait(timeout=5)
                raise TimeoutError("watch process did not shut down after SIGINT")
            if process.returncode != 1:
                raise RuntimeError(
                    f"watch process returned {process.returncode} after SIGINT, expected 1"
                )
            if process.stdout is not None:
                stdout = process.stdout.read()
                process.stdout.close()
                if stdout:
                    raise RuntimeError(f"watch diagnostics leaked to stdout: {stdout!r}")
            if process.stderr is not None:
                process.stderr.close()


if __name__ == "__main__":
    main()
