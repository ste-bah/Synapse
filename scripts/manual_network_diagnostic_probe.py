#!/usr/bin/env python3
"""Supporting network diagnostics for the raw-CDP MCP network tools.

This probe uses the public streamable-HTTP MCP transport, launches a dedicated
raw-CDP Chrome profile, serves a local HTTP/WebSocket fixture, and exercises:
request capture, single-request body readback, route fulfill/abort/continue,
extra headers/User-Agent overrides, HAR record/replay, and WebSocket frame
capture. Its output is supporting diagnostic evidence only. It does not perform
or accept Full State Verification (FSV). Under AGENTS.md D1, an agent must
perform FSV manually through the strict production MCP client and independently
read each physical Source of Truth before and after the trigger.
"""

from __future__ import annotations

import argparse
import base64
import hashlib
import json
import os
import struct
import subprocess
import sys
import tempfile
import threading
import time
import traceback
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path
from typing import Any
from urllib.parse import parse_qs, urlparse

from swarm import McpHttpClient, resolve_bearer_token

BINARY_BODY = bytes([0, 1, 2, 3, 250, 251, 252, 253])
PNG_1X1 = base64.b64decode(
    "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mP8/x8AAwMCAO+/p9sAAAAASUVORK5CYII="
)
DEFAULT_CHROME = r"C:\Program Files\Google\Chrome\Application\chrome.exe"
DEFAULT_MCP_URL = "http://127.0.0.1:7700/mcp"
SERVER_STATE: dict[str, Any] = {"online": True, "hits": [], "ws_frames": []}


def lower_headers(headers: dict[str, str]) -> dict[str, str]:
    return {str(key).lower(): str(value) for key, value in headers.items()}


def send_bytes(
    handler: BaseHTTPRequestHandler,
    status: int,
    body: bytes,
    content_type: str = "application/octet-stream",
    headers: dict[str, str] | None = None,
) -> None:
    headers = headers or {}
    handler.send_response(status)
    handler.send_header("Content-Type", content_type)
    handler.send_header("Content-Length", str(len(body)))
    handler.send_header("Cache-Control", "no-store")
    handler.send_header("Connection", "close")
    for key, value in headers.items():
        handler.send_header(key, value)
    handler.end_headers()
    if body:
        handler.wfile.write(body)


def send_json(
    handler: BaseHTTPRequestHandler,
    status: int,
    obj: dict[str, Any],
    headers: dict[str, str] | None = None,
) -> None:
    body = json.dumps(obj, sort_keys=True).encode("utf-8")
    send_bytes(handler, status, body, "application/json", headers)


def recv_exact(sock: Any, byte_count: int) -> bytes:
    chunks: list[bytes] = []
    remaining = byte_count
    while remaining:
        chunk = sock.recv(remaining)
        if not chunk:
            raise ConnectionError("websocket eof")
        chunks.append(chunk)
        remaining -= len(chunk)
    return b"".join(chunks)


def send_ws_text(sock: Any, text: str) -> None:
    data = text.encode("utf-8")
    if len(data) < 126:
        header = bytes([0x81, len(data)])
    elif len(data) < 65536:
        header = bytes([0x81, 126]) + struct.pack("!H", len(data))
    else:
        header = bytes([0x81, 127]) + struct.pack("!Q", len(data))
    sock.sendall(header + data)


def send_ws_close(sock: Any, code: int = 1000, reason: str = "done") -> None:
    payload = struct.pack("!H", code) + reason.encode("utf-8")
    sock.sendall(bytes([0x88, len(payload)]) + payload)


def read_ws_frame(sock: Any) -> tuple[int, bytes]:
    b1, b2 = recv_exact(sock, 2)
    opcode = b1 & 0x0F
    masked = bool(b2 & 0x80)
    length = b2 & 0x7F
    if length == 126:
        length = struct.unpack("!H", recv_exact(sock, 2))[0]
    elif length == 127:
        length = struct.unpack("!Q", recv_exact(sock, 8))[0]
    mask = recv_exact(sock, 4) if masked else b""
    payload = recv_exact(sock, length) if length else b""
    if masked:
        payload = bytes(byte ^ mask[index % 4] for index, byte in enumerate(payload))
    return opcode, payload


class NetworkDiagnosticHandler(BaseHTTPRequestHandler):
    protocol_version = "HTTP/1.1"
    run_id = ""

    def log_message(self, *_args: Any) -> None:
        return

    def do_GET(self) -> None:
        self.dispatch()

    def do_POST(self) -> None:
        self.dispatch()

    def dispatch(self) -> None:
        parsed = urlparse(self.path)
        path = parsed.path
        query = parse_qs(parsed.query)
        if path == "/ws":
            return self.handle_ws(parsed)
        if path == "/control/offline":
            SERVER_STATE["online"] = False
            return send_json(self, 200, {"ok": True, "online": False})
        if path == "/control/online":
            SERVER_STATE["online"] = True
            return send_json(self, 200, {"ok": True, "online": True})
        if path == "/hits":
            return send_json(
                self,
                200,
                {
                    "hits": SERVER_STATE["hits"],
                    "ws_frames": SERVER_STATE["ws_frames"],
                    "online": SERVER_STATE["online"],
                },
            )

        length = int(self.headers.get("Content-Length", "0") or "0")
        body = self.rfile.read(length) if length else b""
        hit = {
            "method": self.command,
            "path": path,
            "query": parsed.query,
            "headers": lower_headers(dict(self.headers)),
            "body": body.decode("utf-8", "replace"),
        }
        SERVER_STATE["hits"].append(hit)
        if not SERVER_STATE["online"]:
            return send_json(
                self,
                503,
                {"source": "server-offline", "path": path, "run": self.run_id},
            )
        if path in ("/", "/index.html"):
            return self.send_index()
        if path == "/api/data":
            return send_json(
                self,
                200,
                {
                    "source": "server",
                    "path": path,
                    "query": parsed.query,
                    "run": query.get("run", [""])[0],
                    "headers": hit["headers"],
                },
            )
        if path in ("/api/mock", "/api/regex-mock"):
            return send_json(
                self,
                200,
                {
                    "source": "server-unmocked",
                    "path": path,
                    "query": parsed.query,
                    "run": query.get("run", [""])[0],
                },
            )
        if path == "/api/rewrite":
            return send_json(
                self,
                200,
                {
                    "source": "server-original",
                    "path": path,
                    "query": parsed.query,
                    "run": query.get("run", [""])[0],
                },
            )
        if path == "/api/rewritten":
            return send_json(
                self,
                200,
                {
                    "source": "server-rewritten",
                    "path": path,
                    "query": parsed.query,
                    "run": query.get("run", [""])[0],
                    "headers": hit["headers"],
                },
            )
        if path in ("/api/echo", "/api/post-override"):
            return send_json(
                self,
                200,
                {
                    "source": "server",
                    "method": self.command,
                    "path": path,
                    "query": parsed.query,
                    "headers": hit["headers"],
                    "body": hit["body"],
                    "run": query.get("run", [""])[0],
                },
            )
        if path == "/api/not-in-har":
            return send_json(
                self,
                200,
                {"source": "server-not-in-har", "path": path, "query": parsed.query},
            )
        if path == "/text.txt":
            return send_bytes(
                self,
                200,
                f"text-body-{self.run_id}".encode("utf-8"),
                "text/plain",
            )
        if path == "/binary.bin":
            return send_bytes(self, 200, BINARY_BODY, "application/octet-stream")
        if path == "/image.png":
            return send_bytes(self, 200, PNG_1X1, "image/png")
        return send_json(self, 404, {"source": "server", "missing": path})

    def send_index(self) -> None:
        run_id = self.run_id
        html = f"""<!doctype html><meta charset=\"utf-8\"><title>network diagnostic {run_id}</title><body><h1>network diagnostic {run_id}</h1><script>
window.fetchJson = async (url, opts={{}}) => {{ const r = await fetch(url, opts); const text = await r.text(); let body; try {{ body = JSON.parse(text); }} catch {{ body = text; }} return {{ok:r.ok,status:r.status,body,headers:Object.fromEntries(r.headers.entries())}}; }};
window.runBasicFetches = async (run) => {{ const json = await window.fetchJson(`/api/data?case=basic&run=${{run}}`); const text = await fetch(`/text.txt?case=basic&run=${{run}}`).then(r=>r.text()); const binary = Array.from(new Uint8Array(await fetch(`/binary.bin?case=basic&run=${{run}}`).then(r=>r.arrayBuffer()))); let failed = false; try {{ await fetch('http://127.0.0.1:9/synapse-diagnostic-fail'); }} catch (e) {{ failed = true; }} return {{json,text,binary,failed}}; }};
window.loadImage = (src) => new Promise(resolve => {{ const img = new Image(); img.onload = () => resolve({{loaded:true,width:img.naturalWidth,height:img.naturalHeight}}); img.onerror = () => resolve({{loaded:false,error:'image-error'}}); img.src = src; document.body.appendChild(img); }});
window.runWs = (url, msg) => new Promise(resolve => {{ const ws = new WebSocket(url); const events=[]; ws.onopen = () => {{ events.push('open'); ws.send(msg); }}; ws.onmessage = e => events.push('message:' + e.data); ws.onerror = () => events.push('error'); ws.onclose = e => resolve({{events, code:e.code, reason:e.reason, clean:e.wasClean}}); }});
</script></body>""".encode("utf-8")
        send_bytes(self, 200, html, "text/html")

    def handle_ws(self, parsed: Any) -> None:
        key = self.headers.get("Sec-WebSocket-Key")
        if not key:
            return send_json(self, 400, {"error": "missing websocket key"})
        accept = base64.b64encode(
            hashlib.sha1(
                (key + "258EAFA5-E914-47DA-95CA-C5AB0DC85B11").encode("ascii")
            ).digest()
        ).decode("ascii")
        self.send_response(101, "Switching Protocols")
        self.send_header("Upgrade", "websocket")
        self.send_header("Connection", "Upgrade")
        self.send_header("Sec-WebSocket-Accept", accept)
        self.end_headers()
        self.connection.settimeout(5)
        opcode, payload = read_ws_frame(self.connection)
        text = payload.decode("utf-8", "replace")
        SERVER_STATE["ws_frames"].append(
            {
                "direction": "received",
                "opcode": opcode,
                "payload": text,
                "path": parsed.path,
                "query": parsed.query,
            }
        )
        send_ws_text(self.connection, "echo:" + text)
        SERVER_STATE["ws_frames"].append(
            {
                "direction": "sent",
                "opcode": 1,
                "payload": "echo:" + text,
                "path": parsed.path,
                "query": parsed.query,
            }
        )
        send_ws_close(self.connection, 1000, "done")
        time.sleep(0.1)


def assert_true(condition: bool, message: str) -> None:
    if not condition:
        raise AssertionError(message)


def call(
    client: McpHttpClient,
    name: str,
    arguments: dict[str, Any] | None = None,
    timeout_s: float = 60.0,
) -> Any:
    return client.call_tool(name, arguments or {}, timeout_s=timeout_s)


def evaluate(
    client: McpHttpClient,
    target_id: str,
    hwnd: int,
    expression: str,
    timeout_s: float = 60.0,
) -> Any:
    result = call(
        client,
        "browser_evaluate",
        {
            "cdp_target_id": target_id,
            "window_hwnd": hwnd,
            "expression": expression,
            "await_promise": True,
            "return_by_value": True,
        },
        timeout_s,
    )
    return result.get("value")


def network(client: McpHttpClient, target_id: str, hwnd: int, **kwargs: Any) -> Any:
    arguments = {"cdp_target_id": target_id, "window_hwnd": hwnd}
    arguments.update(kwargs)
    return call(client, "browser_network_requests", arguments, 30)


def route(client: McpHttpClient, target_id: str, hwnd: int, **kwargs: Any) -> Any:
    arguments = {"cdp_target_id": target_id, "window_hwnd": hwnd}
    arguments.update(kwargs)
    return call(client, "browser_route", arguments, 30)


def response_status(entry: dict[str, Any]) -> int | None:
    if entry.get("status") is not None:
        return int(entry["status"])
    response = entry.get("response") or {}
    if response.get("status") is not None:
        return int(response["status"])
    return None


def response_body_bytes(detail: dict[str, Any]) -> bytes:
    body = detail.get("response_body") or {}
    text = body.get("body") or ""
    if body.get("base64_encoded"):
        return base64.b64decode(text)
    return text.encode("utf-8")


def wait_entry(
    client: McpHttpClient,
    target_id: str,
    hwnd: int,
    predicate: Any,
    timeout_s: float,
    label: str,
) -> dict[str, Any]:
    deadline = time.time() + timeout_s
    last: Any = None
    while time.time() < deadline:
        last = network(client, target_id, hwnd, limit=1000)
        for entry in last.get("entries", []):
            if predicate(entry):
                return entry
        time.sleep(0.2)
    preview = json.dumps(last, sort_keys=True)[:2000]
    raise AssertionError(f"timed out waiting for {label}; last={preview}")


class Probe:
    def __init__(self, args: argparse.Namespace) -> None:
        self.args = args
        self.run_id = str(int(time.time() * 1000))
        self.work_dir = Path(args.work_dir) if args.work_dir else Path(tempfile.mkdtemp(prefix="synapse-network-diagnostic-"))
        self.work_dir.mkdir(parents=True, exist_ok=True)
        self.result_path = self.work_dir / "result.json"
        self.har_path = self.work_dir / "network.har"
        self.server = ThreadingHTTPServer(("127.0.0.1", 0), NetworkDiagnosticHandler)
        NetworkDiagnosticHandler.run_id = self.run_id
        self.base_url = f"http://127.0.0.1:{self.server.server_port}"
        self.page_url = f"{self.base_url}/index.html?run={self.run_id}"
        self.client: McpHttpClient | None = None
        self.launch: dict[str, Any] | None = None
        self.target_id: str | None = None
        self.hwnd: int | None = None
        self.summary: dict[str, Any] = {
            "status": "started",
            "run_id": self.run_id,
            "work_dir": str(self.work_dir),
            "base_url": self.base_url,
            "har_path": str(self.har_path),
        }

    def out(self, stage: str, **data: Any) -> None:
        payload = {"stage": stage, "run_id": self.run_id, "work_dir": str(self.work_dir)}
        payload.update(data)
        print(json.dumps(payload, sort_keys=True), flush=True)

    def run(self) -> int:
        thread = threading.Thread(target=self.server.serve_forever, daemon=True)
        thread.start()
        try:
            self.out("server_started", base_url=self.base_url)
            self.client = McpHttpClient(
                self.args.mcp_url, resolve_bearer_token(), "synapse-network-diagnostic"
            )
            tools = self.client.initialize()
            tool_names = {tool.get("name") for tool in tools if isinstance(tool, dict)}
            required = {
                "act_launch",
                "browser_evaluate",
                "browser_network_har",
                "browser_network_overrides",
                "browser_network_request",
                "browser_network_requests",
                "browser_network_websockets",
                "browser_route",
                "cdp_close_tab",
                "cdp_open_tab",
            }
            assert_true(
                required.issubset(tool_names),
                "missing required tools: " + ",".join(sorted(required - tool_names)),
            )
            self.launch_raw_chrome()
            self.run_capture_checks()
            self.run_route_checks()
            self.run_override_checks()
            self.run_har_checks()
            self.run_websocket_checks()
            final_routes = route(self.client, self.target_id, self.hwnd, operation="clear")
            assert_true(final_routes.get("route_count") == 0, f"route clear failed: {final_routes}")
            self.summary.update(
                {
                    "status": "passed",
                    "network_cursor_after": network(
                        self.client, self.target_id, self.hwnd, limit=1
                    ).get("next_cursor"),
                    "server_hit_count": len(SERVER_STATE["hits"]),
                }
            )
            self.result_path.write_text(
                json.dumps(self.summary, indent=2, sort_keys=True), encoding="utf-8"
            )
            self.out("passed", result_path=str(self.result_path), har_path=str(self.har_path))
            return 0
        except Exception as exc:
            self.summary.update(
                {
                    "status": "failed",
                    "error": repr(exc),
                    "traceback": traceback.format_exc(),
                }
            )
            self.result_path.write_text(
                json.dumps(self.summary, indent=2, sort_keys=True), encoding="utf-8"
            )
            self.out("failed", result_path=str(self.result_path), error=repr(exc))
            traceback.print_exc()
            return 1
        finally:
            self.cleanup()

    def launch_raw_chrome(self) -> None:
        assert_true(Path(self.args.chrome).exists(), f"Chrome not found: {self.args.chrome}")
        assert self.client is not None
        launch = call(
            self.client,
            "act_launch",
            {
                "target": self.args.chrome,
                "args": [self.page_url],
                "cdp_debug": True,
                "timeout_ms": 30000,
                "wait_for_window_title_regex": ".*",
            },
            60,
        )
        hwnd = launch.get("hwnd")
        assert_true(bool(hwnd), f"act_launch did not return hwnd: {launch}")
        assert_true(bool(launch.get("cdp_endpoint")), f"act_launch missing cdp endpoint: {launch}")
        tab = call(self.client, "cdp_open_tab", {"window_hwnd": hwnd, "url": self.page_url}, 60)
        target_id = tab.get("cdp_target_id")
        assert_true(
            bool(target_id and not str(target_id).startswith("chrome-tab:")),
            f"expected raw CDP target, got {tab}",
        )
        self.launch = launch
        self.hwnd = int(hwnd)
        self.target_id = str(target_id)
        self.summary.update(
            {
                "raw_chrome_pid": launch.get("pid"),
                "raw_chrome_hwnd": hwnd,
                "cdp_endpoint": launch.get("cdp_endpoint"),
                "cdp_target_id": target_id,
            }
        )
        self.out("chrome_ready", pid=launch.get("pid"), hwnd=hwnd, target=target_id)
        route(self.client, self.target_id, self.hwnd, operation="clear")

    def run_capture_checks(self) -> None:
        assert self.client is not None and self.target_id is not None and self.hwnd is not None
        arm = network(self.client, self.target_id, self.hwnd, limit=5)
        start_cursor = int(arm.get("next_cursor") or 0) + 1
        assert_true(arm.get("cdp_target_id") == self.target_id, "network capture target mismatch")
        basic = evaluate(
            self.client,
            self.target_id,
            self.hwnd,
            f"window.runBasicFetches('{self.run_id}')",
            60,
        )
        assert_true(
            basic["json"]["status"] == 200 and basic["json"]["body"]["source"] == "server",
            f"basic json fetch failed: {basic}",
        )
        assert_true(basic["text"] == f"text-body-{self.run_id}", f"text mismatch: {basic}")
        assert_true(basic["binary"] == list(BINARY_BODY), f"binary mismatch: {basic}")
        assert_true(basic["failed"] is True, f"failed fetch did not fail: {basic}")

        basic_entry = wait_entry(
            self.client,
            self.target_id,
            self.hwnd,
            lambda entry: f"/api/data?case=basic&run={self.run_id}" in (entry.get("url") or "")
            and entry.get("loading_finished")
            and response_status(entry) == 200,
            12,
            "basic api/data",
        )
        text_entry = wait_entry(
            self.client,
            self.target_id,
            self.hwnd,
            lambda entry: f"/text.txt?case=basic&run={self.run_id}" in (entry.get("url") or "")
            and entry.get("loading_finished"),
            12,
            "text request",
        )
        binary_entry = wait_entry(
            self.client,
            self.target_id,
            self.hwnd,
            lambda entry: f"/binary.bin?case=basic&run={self.run_id}" in (entry.get("url") or "")
            and entry.get("loading_finished"),
            12,
            "binary request",
        )
        fail_entry = wait_entry(
            self.client,
            self.target_id,
            self.hwnd,
            lambda entry: "127.0.0.1:9/synapse-diagnostic-fail" in (entry.get("url") or "")
            and entry.get("loading_failed"),
            12,
            "failed request",
        )
        filtered = network(
            self.client,
            self.target_id,
            self.hwnd,
            since_seq=start_cursor,
            limit=50,
            url_contains="/api/data?case=basic",
            status_min=200,
            status_max=200,
            resource_type="Fetch",
        )
        assert_true(filtered.get("returned", 0) >= 1, f"filtered request missing: {filtered}")
        regexed = network(
            self.client,
            self.target_id,
            self.hwnd,
            url_regex=f"case=basic.*run={self.run_id}",
            limit=50,
        )
        assert_true(regexed.get("returned", 0) >= 2, f"regex requests missing: {regexed}")

        detail_json = call(
            self.client,
            "browser_network_request",
            {
                "cdp_target_id": self.target_id,
                "window_hwnd": self.hwnd,
                "request_id": basic_entry["request_id"],
                "include_body": True,
                "include_post_data": True,
            },
            30,
        )
        assert_true(
            b'"source": "server"' in response_body_bytes(detail_json),
            "json response body readback missing server source",
        )
        detail_bin = call(
            self.client,
            "browser_network_request",
            {
                "cdp_target_id": self.target_id,
                "window_hwnd": self.hwnd,
                "request_id": binary_entry["request_id"],
                "include_body": True,
            },
            30,
        )
        assert_true(
            response_body_bytes(detail_bin) == BINARY_BODY,
            f"binary response body mismatch: {detail_bin.get('response_body')}",
        )
        self.summary.update(
            {
                "basic_request_id": basic_entry["request_id"],
                "text_request_id": text_entry["request_id"],
                "binary_request_id": binary_entry["request_id"],
                "failed_request_id": fail_entry["request_id"],
            }
        )
        self.out("capture_passed", basic_request_id=basic_entry["request_id"])

    def run_route_checks(self) -> None:
        assert self.client is not None and self.target_id is not None and self.hwnd is not None
        route_result = route(
            self.client,
            self.target_id,
            self.hwnd,
            operation="add_fulfill",
            route_id="fulfill-json",
            url="*/api/mock*",
            status=203,
            headers=[
                {"name": "Content-Type", "value": "application/json"},
                {"name": "X-Synapse-Mock", "value": self.run_id},
            ],
            body=json.dumps({"source": "mocked", "run": self.run_id, "case": "fulfill"}),
        )
        assert_true(
            route_result.get("route_count", 0) >= 1
            and route_result.get("fetch_status", {}).get("fetch_armed"),
            f"fulfill route not armed: {route_result}",
        )
        mocked = evaluate(
            self.client,
            self.target_id,
            self.hwnd,
            f"window.fetchJson('/api/mock?case=fulfill&run={self.run_id}')",
            30,
        )
        assert_true(
            mocked["status"] == 203
            and mocked["body"]["source"] == "mocked"
            and mocked["body"]["run"] == self.run_id,
            f"fulfill did not mock: {mocked}",
        )
        passthrough = evaluate(
            self.client,
            self.target_id,
            self.hwnd,
            f"window.fetchJson('/api/data?case=passthrough&run={self.run_id}')",
            30,
        )
        assert_true(
            passthrough["status"] == 200 and passthrough["body"]["source"] == "server",
            f"unmatched request did not continue by default: {passthrough}",
        )
        route(self.client, self.target_id, self.hwnd, operation="remove", route_id="fulfill-json")
        restored = evaluate(
            self.client,
            self.target_id,
            self.hwnd,
            f"window.fetchJson('/api/mock?case=restored&run={self.run_id}')",
            30,
        )
        assert_true(
            restored["status"] == 200 and restored["body"]["source"] == "server-unmocked",
            f"removed fulfill route did not restore: {restored}",
        )
        route(
            self.client,
            self.target_id,
            self.hwnd,
            operation="add_fulfill",
            route_id="fulfill-regex",
            match_kind="regex",
            url=r".*/api/regex-mock.*",
            status=204,
            headers=[{"name": "X-Synapse-Regex", "value": self.run_id}],
            body="",
        )
        regex_mocked = evaluate(
            self.client,
            self.target_id,
            self.hwnd,
            f"fetch('/api/regex-mock?case=regex&run={self.run_id}').then(r=>({{status:r.status,header:r.headers.get('x-synapse-regex')}}))",
            30,
        )
        assert_true(
            regex_mocked["status"] == 204 and regex_mocked["header"] == self.run_id,
            f"regex fulfill failed: {regex_mocked}",
        )
        route(self.client, self.target_id, self.hwnd, operation="remove", route_id="fulfill-regex")
        self.out("fulfill_passed")

        route(
            self.client,
            self.target_id,
            self.hwnd,
            operation="add_abort",
            route_id="abort-image",
            url="*/image.png*",
            resource_type="Image",
            error_reason="blocked_by_client",
        )
        img_blocked = evaluate(
            self.client,
            self.target_id,
            self.hwnd,
            f"window.loadImage('/image.png?case=abort&run={self.run_id}')",
            30,
        )
        assert_true(img_blocked["loaded"] is False, f"image abort did not fail: {img_blocked}")
        aborted_entry = wait_entry(
            self.client,
            self.target_id,
            self.hwnd,
            lambda entry: f"/image.png?case=abort&run={self.run_id}" in (entry.get("url") or "")
            and entry.get("loading_failed"),
            12,
            "aborted image",
        )
        route(self.client, self.target_id, self.hwnd, operation="remove", route_id="abort-image")
        img_restored = evaluate(
            self.client,
            self.target_id,
            self.hwnd,
            f"window.loadImage('/image.png?case=restore&run={self.run_id}')",
            30,
        )
        assert_true(
            img_restored["loaded"] is True and img_restored["width"] == 1,
            f"image route removal did not restore load: {img_restored}",
        )
        self.out("abort_passed", aborted_request_id=aborted_entry["request_id"])

        route(
            self.client,
            self.target_id,
            self.hwnd,
            operation="add_continue",
            route_id="continue-header",
            url="*/api/echo?case=continue-header*",
            continue_headers=[{"name": "X-Synapse-Route", "value": self.run_id}],
        )
        header_echo = evaluate(
            self.client,
            self.target_id,
            self.hwnd,
            f"window.fetchJson('/api/echo?case=continue-header&run={self.run_id}')",
            30,
        )
        assert_true(
            header_echo["body"]["headers"].get("x-synapse-route") == self.run_id,
            f"continue header override missing: {header_echo}",
        )
        route(self.client, self.target_id, self.hwnd, operation="remove", route_id="continue-header")
        route(
            self.client,
            self.target_id,
            self.hwnd,
            operation="add_continue",
            route_id="continue-url",
            url="*/api/rewrite?case=continue-url*",
            continue_url=f"{self.base_url}/api/rewritten?case=continue-url&run={self.run_id}",
        )
        rewrite = evaluate(
            self.client,
            self.target_id,
            self.hwnd,
            f"window.fetchJson('/api/rewrite?case=continue-url&run={self.run_id}')",
            30,
        )
        assert_true(
            rewrite["body"]["source"] == "server-rewritten"
            and rewrite["body"]["path"] == "/api/rewritten",
            f"continue url rewrite failed: {rewrite}",
        )
        route(self.client, self.target_id, self.hwnd, operation="remove", route_id="continue-url")
        post_payload = json.dumps({"rewritten": True, "run": self.run_id}, sort_keys=True)
        route(
            self.client,
            self.target_id,
            self.hwnd,
            operation="add_continue",
            route_id="continue-post",
            url="*/api/post-override?case=continue-post*",
            continue_method="POST",
            continue_post_data=post_payload,
            continue_headers=[
                {"name": "Content-Type", "value": "application/json"},
                {"name": "X-Synapse-Post", "value": self.run_id},
            ],
        )
        post_echo = evaluate(
            self.client,
            self.target_id,
            self.hwnd,
            f"window.fetchJson('/api/post-override?case=continue-post&run={self.run_id}')",
            30,
        )
        assert_true(
            post_echo["body"]["method"] == "POST"
            and post_echo["body"]["body"] == post_payload
            and post_echo["body"]["headers"].get("x-synapse-post") == self.run_id,
            f"continue post/method/header failed: {post_echo}",
        )
        route(self.client, self.target_id, self.hwnd, operation="remove", route_id="continue-post")
        self.out("continue_passed")

    def run_override_checks(self) -> None:
        assert self.client is not None and self.target_id is not None and self.hwnd is not None
        ua = f"SynapseNetworkDiagnostic/{self.run_id}"
        override = call(
            self.client,
            "browser_network_overrides",
            {
                "cdp_target_id": self.target_id,
                "window_hwnd": self.hwnd,
                "operation": "set",
                "headers": [{"name": "X-Synapse-Override", "value": self.run_id}],
                "user_agent": ua,
            },
            30,
        )
        assert_true(
            override.get("override_active")
            and override.get("header_count") == 1
            and override.get("user_agent") == ua,
            f"override set readback mismatch: {override}",
        )
        ua_read = evaluate(self.client, self.target_id, self.hwnd, "navigator.userAgent", 30)
        override_echo = evaluate(
            self.client,
            self.target_id,
            self.hwnd,
            f"window.fetchJson('/api/echo?case=override&run={self.run_id}')",
            30,
        )
        assert_true(ua_read == ua, f"navigator UA not overridden: {ua_read}")
        assert_true(
            override_echo["body"]["headers"].get("x-synapse-override") == self.run_id
            and override_echo["body"]["headers"].get("user-agent") == ua,
            f"override headers missing: {override_echo}",
        )
        cleared = call(
            self.client,
            "browser_network_overrides",
            {"cdp_target_id": self.target_id, "window_hwnd": self.hwnd, "operation": "clear"},
            30,
        )
        assert_true(
            cleared.get("cleared") and not cleared.get("override_active"),
            f"override clear failed: {cleared}",
        )
        self.out("overrides_passed")

    def run_har_checks(self) -> None:
        assert self.client is not None and self.target_id is not None and self.hwnd is not None
        har_url = f"/api/data?case=har&run={self.run_id}"
        seed = evaluate(
            self.client,
            self.target_id,
            self.hwnd,
            f"window.fetchJson('{har_url}')",
            30,
        )
        assert_true(seed["body"]["source"] == "server", f"HAR seed fetch failed: {seed}")
        har_entry = wait_entry(
            self.client,
            self.target_id,
            self.hwnd,
            lambda entry: har_url in (entry.get("url") or "")
            and entry.get("loading_finished")
            and response_status(entry) == 200,
            12,
            "har seed",
        )
        har_record = call(
            self.client,
            "browser_network_har",
            {
                "cdp_target_id": self.target_id,
                "window_hwnd": self.hwnd,
                "operation": "record",
                "path": str(self.har_path),
                "include_bodies": True,
                "url_contains": self.base_url,
                "limit": 1000,
            },
            60,
        )
        assert_true(
            har_record.get("recorded_entry_count", 0) >= 6
            and self.har_path.exists()
            and self.har_path.stat().st_size > 0,
            f"HAR record failed: {har_record}",
        )
        har_json = json.loads(self.har_path.read_text(encoding="utf-8"))
        assert_true(
            any(
                har_url in (entry.get("request", {}).get("url") or "")
                for entry in har_json["log"]["entries"]
            ),
            "HAR missing seed URL",
        )
        har_replay = call(
            self.client,
            "browser_network_har",
            {
                "cdp_target_id": self.target_id,
                "window_hwnd": self.hwnd,
                "operation": "replay",
                "path": str(self.har_path),
                "missing_policy": "abort",
                "clear_existing_replay": True,
            },
            60,
        )
        assert_true(
            har_replay.get("replay_route_count", 0) >= har_record.get("recorded_entry_count", 0)
            and har_replay.get("missing_abort_route_installed"),
            f"HAR replay did not install routes: {har_replay}",
        )
        SERVER_STATE["online"] = False
        replayed = evaluate(
            self.client,
            self.target_id,
            self.hwnd,
            f"window.fetchJson('{har_url}')",
            30,
        )
        assert_true(
            replayed["status"] == 200 and replayed["body"]["source"] == "server",
            f"HAR replay exact request failed offline: {replayed}",
        )
        missing = evaluate(
            self.client,
            self.target_id,
            self.hwnd,
            f"(async () => {{ try {{ await fetch('/api/not-in-har?run={self.run_id}'); return {{failed:false}}; }} catch(e) {{ return {{failed:true,name:e.name,message:String(e.message)}}; }} }})()",
            30,
        )
        assert_true(missing["failed"] is True, f"HAR missing abort did not fail: {missing}")
        SERVER_STATE["online"] = True
        har_clear = call(
            self.client,
            "browser_network_har",
            {
                "cdp_target_id": self.target_id,
                "window_hwnd": self.hwnd,
                "operation": "clear_replay",
            },
            30,
        )
        assert_true(har_clear.get("cleared_replay_route_count", 0) >= 1, f"HAR clear failed: {har_clear}")
        self.summary.update(
            {
                "har_entries": har_record.get("recorded_entry_count"),
                "har_bytes": self.har_path.stat().st_size,
                "har_seed_request_id": har_entry["request_id"],
            }
        )
        self.out("har_passed", har_entries=har_record.get("recorded_entry_count"))

    def run_websocket_checks(self) -> None:
        assert self.client is not None and self.target_id is not None and self.hwnd is not None
        call(
            self.client,
            "browser_network_websockets",
            {"cdp_target_id": self.target_id, "window_hwnd": self.hwnd, "limit": 5},
            30,
        )
        ws_url = f"ws://127.0.0.1:{self.server.server_port}/ws?run={self.run_id}"
        page = evaluate(
            self.client,
            self.target_id,
            self.hwnd,
            f"window.runWs('{ws_url}', 'hello-{self.run_id}')",
            30,
        )
        assert_true(
            page["code"] == 1000
            and any(event == "message:echo:hello-" + self.run_id for event in page["events"]),
            f"page websocket failed: {page}",
        )
        ws_entry: dict[str, Any] | None = None
        last_ws: Any = None
        deadline = time.time() + 12
        while time.time() < deadline:
            last_ws = call(
                self.client,
                "browser_network_websockets",
                {
                    "cdp_target_id": self.target_id,
                    "window_hwnd": self.hwnd,
                    "url_contains": f"run={self.run_id}",
                    "limit": 20,
                },
                30,
            )
            for entry in last_ws.get("entries", []):
                payloads = [frame.get("payload_data") for frame in entry.get("frames", [])]
                if (
                    entry.get("sent_frame_count", 0) >= 1
                    and entry.get("received_frame_count", 0) >= 1
                    and "hello-" + self.run_id in payloads
                    and "echo:hello-" + self.run_id in payloads
                ):
                    ws_entry = entry
                    break
            if ws_entry:
                break
            time.sleep(0.2)
        assert_true(ws_entry is not None, f"websocket capture missing frames: {last_ws}")
        assert ws_entry is not None
        self.summary.update(
            {
                "websocket_request_id": ws_entry.get("request_id"),
                "websocket_sent_frames": ws_entry.get("sent_frame_count"),
                "websocket_received_frames": ws_entry.get("received_frame_count"),
                "websocket_closed": ws_entry.get("closed"),
                "websocket_close_code": ws_entry.get("close_code"),
            }
        )
        self.out(
            "websocket_passed",
            sent=ws_entry.get("sent_frame_count"),
            received=ws_entry.get("received_frame_count"),
            closed=ws_entry.get("closed"),
            close_code=ws_entry.get("close_code"),
        )

    def cleanup(self) -> None:
        SERVER_STATE["online"] = True
        if self.client and self.target_id and self.hwnd:
            try:
                call(
                    self.client,
                    "browser_network_overrides",
                    {
                        "cdp_target_id": self.target_id,
                        "window_hwnd": self.hwnd,
                        "operation": "clear",
                    },
                    10,
                )
            except Exception:
                pass
            try:
                route(self.client, self.target_id, self.hwnd, operation="clear")
            except Exception:
                pass
            try:
                call(self.client, "cdp_close_tab", {"cdp_target_id": self.target_id}, 10)
            except Exception:
                pass
        if self.launch and self.launch.get("pid"):
            try:
                subprocess.run(
                    ["taskkill", "/PID", str(self.launch["pid"]), "/T", "/F"],
                    stdout=subprocess.DEVNULL,
                    stderr=subprocess.DEVNULL,
                    timeout=10,
                    check=False,
                )
            except Exception:
                pass
        try:
            self.server.shutdown()
            self.server.server_close()
        except Exception:
            pass


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--mcp-url", default=DEFAULT_MCP_URL)
    parser.add_argument("--chrome", default=os.environ.get("CHROME_EXE", DEFAULT_CHROME))
    parser.add_argument("--work-dir")
    return parser.parse_args()


def main() -> int:
    return Probe(parse_args()).run()


if __name__ == "__main__":
    sys.exit(main())
