#!/usr/bin/env python3
"""Minimal stdio language server for aldwin-tools' LspClient tests.

Speaks `Content-Length` framed JSON-RPC and just enough LSP to show what the
client sent: `textDocument/hover` answers with the text the server currently
holds for the document and how many `didOpen`/`didChange` it took to get
there, and `test/die` exits without answering.
"""
import json
import sys


def read_message():
    length = None
    while True:
        line = sys.stdin.buffer.readline()
        if not line:
            return None
        line = line.strip()
        if not line:
            break
        name, _, value = line.partition(b":")
        if name.lower() == b"content-length":
            length = int(value)
    return json.loads(sys.stdin.buffer.read(length))


def send(message):
    body = json.dumps(message).encode()
    sys.stdout.buffer.write(b"Content-Length: %d\r\n\r\n" % len(body) + body)
    sys.stdout.buffer.flush()


def main():
    documents = {}
    counts = {"opens": 0, "changes": 0}
    while (request := read_message()) is not None:
        method = request.get("method")
        req_id = request.get("id")
        params = request.get("params") or {}

        if method == "textDocument/didOpen":
            counts["opens"] += 1
            documents[params["textDocument"]["uri"]] = params["textDocument"]["text"]
        elif method == "textDocument/didChange":
            counts["changes"] += 1
            documents[params["textDocument"]["uri"]] = params["contentChanges"][-1]["text"]
        elif method == "textDocument/hover":
            text = documents.get(params["textDocument"]["uri"], "")
            send({"jsonrpc": "2.0", "id": req_id, "result": {"contents": text, **counts}})
        elif method == "test/die":
            sys.exit(0)
        elif method == "exit":
            return
        elif req_id is not None:
            send({"jsonrpc": "2.0", "id": req_id, "result": None})


if __name__ == "__main__":
    main()
