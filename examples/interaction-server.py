#!/usr/bin/env python3
"""Local-only GET/POST echo server for Olive's interaction demo."""
from html import escape
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path
from urllib.parse import parse_qsl, urlsplit
import argparse


class Handler(BaseHTTPRequestHandler):
    def do_GET(self):
        url = urlsplit(self.path)
        if url.path == "/":
            self.respond(Path(__file__).with_name("interaction.html").read_bytes())
        elif url.path == "/submit":
            self.echo("GET", url.query)
        else:
            self.send_error(404)

    def do_POST(self):
        if urlsplit(self.path).path != "/submit":
            self.send_error(404)
            return
        try:
            length = int(self.headers.get("Content-Length", "0"))
        except ValueError:
            self.send_error(400)
            return
        if not 0 <= length <= 64 * 1024:
            self.send_error(413)
            return
        self.echo("POST", self.rfile.read(length).decode("utf-8"))

    def echo(self, method, data):
        rows = "".join(
            f"<li><strong>{escape(name)}</strong>: {escape(value)}</li>"
            for name, value in parse_qsl(data, keep_blank_values=True)
        )
        html = (
            f"<!doctype html><meta charset=utf-8><title>{method} received</title>"
            f"<h1>{method} received</h1><p>Your form values, in submission order:</p>"
            f"<ul>{rows or '<li>No values</li>'}</ul>"
            "<p><a href='/'>Back to the interaction demo</a></p>"
            "<p>Reload uses GET; it does not resend the POST body.</p>"
        )
        self.respond(html.encode("utf-8"))

    def respond(self, body):
        self.send_response(200)
        self.send_header("Content-Type", "text/html; charset=utf-8")
        self.send_header("Content-Length", str(len(body)))
        self.end_headers()
        self.wfile.write(body)

    def log_message(self, *_args):
        pass


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--port", type=int, default=8000)
    args = parser.parse_args()
    server = ThreadingHTTPServer(("127.0.0.1", args.port), Handler)
    print(f"Open http://localhost:{args.port} in Olive. Ctrl+C stops the demo.", flush=True)
    try:
        server.serve_forever()
    except KeyboardInterrupt:
        pass
    finally:
        server.server_close()
