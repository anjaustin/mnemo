#!/usr/bin/env python3
"""Tiny OpenAI-compatible chat completion mock for dashboard/browser smoke tests."""

from __future__ import annotations

import json
import os
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer


HOST = os.environ.get("MNEMO_MOCK_OPENAI_HOST", "127.0.0.1")
PORT = int(os.environ.get("MNEMO_MOCK_OPENAI_PORT", "18080"))


class Handler(BaseHTTPRequestHandler):
    def do_POST(self):  # noqa: N802
        length = int(self.headers.get("Content-Length", "0"))
        if length:
            self.rfile.read(length)

        if self.path != "/v1/chat/completions":
            self.send_response(404)
            self.end_headers()
            return

        payload = {
            "id": "chatcmpl-mnemo-smoke",
            "object": "chat.completion",
            "choices": [
                {
                    "index": 0,
                    "message": {
                        "role": "assistant",
                        "content": json.dumps({"entities": [], "relationships": []}),
                    },
                }
            ],
        }
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
