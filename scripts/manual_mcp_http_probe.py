#!/usr/bin/env python3
"""Manual MCP-over-Streamable-HTTP probe for the live synapse-mcp daemon.

Speaks to the running HTTP daemon exactly as the wired clients (Claude Code
/mcp, Codex) do: JSON-RPC 2.0 over POST /mcp with bearer auth and the
MCP-Session-Id header. Performs the initialize handshake, fetches tools/list,
VALIDATES that every requested tool is present with a closed object input
schema (client-parity per AGENTS.md D1: a caller that skips tools/list schema
validation can mask total tool-surface outages), then calls the requested
tools and prints raw results for agent-directed runtime inspection.

This helper does not perform or replace manual FSV. The agent must still
define the Source of Truth, read it before the trigger, perform the tool
call, and read the separate Source of Truth afterward.

Usage:
    python3 manual_mcp_http_probe.py <url> <tool1> [tool2 ...]
Tool spec: name or name:{json-args}
Bearer token is read from %APPDATA%/synapse/token.txt or SYNAPSE_BEARER_TOKEN.
"""

import json
import os
import sys
import urllib.request


def bearer_token() -> str:
    env = os.environ.get("SYNAPSE_BEARER_TOKEN")
    if env:
        return env.strip()
    appdata = os.environ.get("APPDATA")
    if not appdata:
        raise SystemExit("no SYNAPSE_BEARER_TOKEN and no APPDATA to locate token.txt")
    path = os.path.join(appdata, "synapse", "token.txt")
    with open(path, encoding="utf-8") as fh:
        return fh.read().strip()


def parse_body(raw: bytes, content_type: str):
    """The streamable-HTTP transport may answer application/json or a
    text/event-stream body containing data: lines."""
    text = raw.decode("utf-8", errors="replace")
    if "text/event-stream" in content_type:
        messages = []
        for line in text.splitlines():
            if line.startswith("data:"):
                payload = line[len("data:"):].strip()
                if payload:
                    messages.append(json.loads(payload))
        if not messages:
            raise SystemExit(f"SSE body contained no data lines: {text[:500]}")
        return messages[-1]
    return json.loads(text)


class HttpMcpClient:
    def __init__(self, url: str, token: str):
        self.url = url
        self.token = token
        self.session_id = None
        self.next_id = 1

    def rpc(self, method: str, params):
        request_id = self.next_id
        self.next_id += 1
        body = {"jsonrpc": "2.0", "id": request_id, "method": method}
        if params is not None:
            body["params"] = params
        data = json.dumps(body).encode("utf-8")
        headers = {
            "Authorization": f"Bearer {self.token}",
            "Content-Type": "application/json",
            "Accept": "application/json, text/event-stream",
        }
        if self.session_id:
            headers["MCP-Session-Id"] = self.session_id
        request = urllib.request.Request(self.url, data=data, headers=headers)
        with urllib.request.urlopen(request, timeout=120) as response:
            session = response.headers.get("MCP-Session-Id")
            if session:
                self.session_id = session
            message = parse_body(response.read(), response.headers.get("Content-Type", ""))
        if message.get("id") != request_id:
            raise SystemExit(f"response id mismatch for {method}: {message}")
        return message

    def notify(self, method: str):
        body = {"jsonrpc": "2.0", "method": method}
        data = json.dumps(body).encode("utf-8")
        headers = {
            "Authorization": f"Bearer {self.token}",
            "Content-Type": "application/json",
            "Accept": "application/json, text/event-stream",
        }
        if self.session_id:
            headers["MCP-Session-Id"] = self.session_id
        request = urllib.request.Request(self.url, data=data, headers=headers)
        with urllib.request.urlopen(request, timeout=30) as response:
            response.read()


def validate_tool_schema(tool: dict) -> None:
    """Mirror the strict-client expectations the schema_sanitize gate exists
    for: a usable tool advertises a closed object inputSchema, never a bare
    boolean schema."""
    name = tool.get("name", "<unnamed>")
    schema = tool.get("inputSchema")
    if not isinstance(schema, dict):
        raise SystemExit(f"tool {name}: inputSchema is not an object schema: {schema!r}")
    if schema.get("type") != "object":
        raise SystemExit(f"tool {name}: inputSchema.type != object: {schema.get('type')!r}")
    if schema.get("additionalProperties") not in (False, None):
        raise SystemExit(
            f"tool {name}: inputSchema.additionalProperties is open: "
            f"{schema.get('additionalProperties')!r}"
        )


def main() -> None:
    if len(sys.argv) < 3:
        print("usage: manual_mcp_http_probe.py <url> <tool[:jsonargs]> ...", file=sys.stderr)
        raise SystemExit(2)
    url = sys.argv[1]
    tool_specs = sys.argv[2:]
    client = HttpMcpClient(url, bearer_token())

    init = client.rpc(
        "initialize",
        {
            "protocolVersion": "2025-03-26",
            "capabilities": {},
            "clientInfo": {"name": "manual_mcp_http_probe", "version": "1.0"},
        },
    )
    server_info = init.get("result", {}).get("serverInfo", {})
    print(f"probe=initialize session={client.session_id} server={json.dumps(server_info)}")
    client.notify("notifications/initialized")

    listed = client.rpc("tools/list", {})
    tools = listed.get("result", {}).get("tools", [])
    by_name = {tool.get("name"): tool for tool in tools}
    print(f"probe=tools/list count={len(tools)}")

    for spec in tool_specs:
        name, _, raw_args = spec.partition(":")
        arguments = json.loads(raw_args) if raw_args else {}
        tool = by_name.get(name)
        if tool is None:
            raise SystemExit(f"tool {name} is NOT in tools/list ({len(tools)} tools)")
        validate_tool_schema(tool)
        print(f"probe=schema_validated tool={name}")
        result = client.rpc("tools/call", {"name": name, "arguments": arguments})
        print(f"probe=tools/call tool={name} result={json.dumps(result.get('result', result))}")


if __name__ == "__main__":
    main()
