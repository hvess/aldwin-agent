#!/usr/bin/env python3
"""Minimal stdio MCP server for mjolnir-tools' McpBridge integration test.

Speaks just enough of the protocol (newline-delimited JSON-RPC 2.0) to
exercise the real subprocess + rmcp client wiring: initialize, tools/list,
tools/call for one tool ("echo"), and an error result for any other tool
name. Not a general-purpose test double — just what this one test needs.
"""
import json
import sys


def send(message):
    sys.stdout.write(json.dumps(message) + "\n")
    sys.stdout.flush()


def main():
    for line in sys.stdin:
        line = line.strip()
        if not line:
            continue
        request = json.loads(line)
        method = request.get("method")
        req_id = request.get("id")

        if method == "initialize":
            client_version = request["params"].get("protocolVersion", "2025-11-25")
            send({
                "jsonrpc": "2.0",
                "id": req_id,
                "result": {
                    "protocolVersion": client_version,
                    "capabilities": {"tools": {}},
                    "serverInfo": {"name": "fake-mcp-server", "version": "0.1.0"},
                },
            })
        elif method == "notifications/initialized":
            pass  # no response expected
        elif method == "tools/list":
            send({
                "jsonrpc": "2.0",
                "id": req_id,
                "result": {
                    "tools": [
                        {
                            "name": "echo",
                            "description": "Echoes the given text back.",
                            "inputSchema": {
                                "type": "object",
                                "properties": {"text": {"type": "string"}},
                                "required": ["text"],
                            },
                        }
                    ]
                },
            })
        elif method == "tools/call":
            params = request.get("params", {})
            name = params.get("name")
            arguments = params.get("arguments") or {}
            if name == "echo":
                send({
                    "jsonrpc": "2.0",
                    "id": req_id,
                    "result": {
                        "content": [{"type": "text", "text": arguments.get("text", "")}],
                        "isError": False,
                    },
                })
            else:
                send({
                    "jsonrpc": "2.0",
                    "id": req_id,
                    "result": {
                        "content": [{"type": "text", "text": f"no such tool: {name}"}],
                        "isError": True,
                    },
                })
        elif req_id is not None:
            send({"jsonrpc": "2.0", "id": req_id, "error": {"code": -32601, "message": f"method not found: {method}"}})
        # else: an unhandled notification — ignore.


if __name__ == "__main__":
    main()
