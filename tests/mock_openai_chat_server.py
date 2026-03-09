#!/usr/bin/env python3
"""Tiny OpenAI-compatible chat completion mock for dashboard/browser smoke tests."""

from __future__ import annotations

import json
import os
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer


HOST = os.environ.get("MNEMO_MOCK_OPENAI_HOST", "127.0.0.1")
PORT = int(os.environ.get("MNEMO_MOCK_OPENAI_PORT", "18080"))


class Handler(BaseHTTPRequestHandler):
    def do_GET(self):  # noqa: N802
        if self.path == "/health":
            self.send_response(200)
            self.send_header("Content-Type", "application/json")
            body = b'{"status":"ok"}'
            self.send_header("Content-Length", str(len(body)))
            self.end_headers()
            self.wfile.write(body)
            return
        self.send_response(404)
        self.end_headers()

    def do_POST(self):  # noqa: N802
        length = int(self.headers.get("Content-Length", "0"))
        raw_body = self.rfile.read(length) if length else b""

        if self.path == "/v1/chat/completions":
            payload = {
                "id": "chatcmpl-mnemo-smoke",
                "object": "chat.completion",
                "choices": [
                    {
                        "index": 0,
                        "message": {
                            "role": "assistant",
                            "content": json.dumps(
                                {"entities": [], "relationships": []}
                            ),
                        },
                    }
                ],
            }
        elif self.path == "/v1/embeddings":
            # Parse request to determine how many inputs were sent
            try:
                req = json.loads(raw_body)
                inputs = req.get("input", [])
                if isinstance(inputs, str):
                    inputs = [inputs]
            except (json.JSONDecodeError, AttributeError):
                inputs = [""]
            # Return zero vectors of dimension 384
            dim = 384
            payload = {
                "object": "list",
                "data": [
                    {"object": "embedding", "index": i, "embedding": [0.0] * dim}
                    for i in range(len(inputs))
                ],
                "model": "mock-embedding",
                "usage": {"prompt_tokens": 0, "total_tokens": 0},
            }
        else:
            self.send_response(404)
            self.end_headers()
            return

        body = json.dumps(payload).encode("utf-8")
        self.send_response(200)
        self.send_header("Content-Type", "application/json")
        self.send_header("Content-Length", str(len(body)))
        self.end_headers()
        self.wfile.write(body)

    def log_message(self, fmt, *args):  # noqa: A003
        return


def main() -> int:
    server = ThreadingHTTPServer((HOST, PORT), Handler)
    print(
        f"mock openai chat server listening on http://{HOST}:{PORT}/v1/chat/completions"
    )
    try:
        server.serve_forever()
    except KeyboardInterrupt:
        pass
    finally:
        server.server_close()
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
