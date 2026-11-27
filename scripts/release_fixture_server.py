#!/usr/bin/env python3
"""Serve a fake release directory over HTTP for the plugin launcher tests.

Answers Range requests and can cap the transfer rate, and logs each request
with its Range header, so a test can check that an interrupted download
resumes where it stopped. Writes the port it listens on to --port-file.
"""

import argparse
import http.server
import os
import time


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("--dir", required=True)
    parser.add_argument("--port-file", required=True)
    parser.add_argument("--log", required=True)
    parser.add_argument("--rate", type=int, default=0, help="bytes per second, 0 for no cap")
    parser.add_argument("--no-ranges", action="store_true", help="ignore Range and send the whole file")
    args = parser.parse_args()
    # Requests can only name a file listed here, never build a path.
    root = os.path.realpath(args.dir)
    files = {entry.name: entry.path for entry in os.scandir(root) if entry.is_file()}

    class Handler(http.server.BaseHTTPRequestHandler):
        protocol_version = "HTTP/1.1"

        def log_message(self, *_args) -> None:
            pass

        def do_HEAD(self) -> None:
            self.serve(body=False)

        def do_GET(self) -> None:
            self.serve(body=True)

        def empty(self, status: int, headers: dict[str, str] | None = None) -> None:
            self.send_response(status)
            for key, value in (headers or {}).items():
                self.send_header(key, value)
            self.send_header("Content-Length", "0")
            self.end_headers()

        def serve(self, body: bool) -> None:
            ranges = self.headers.get("Range", "")
            with open(args.log, "a", encoding="utf-8") as log:
                log.write(f"{self.command} {self.path} {ranges}\n")
            path = files.get(self.path.split("?", 1)[0].rsplit("/", 1)[-1])
            if path is None:
                self.empty(404)
                return
            size = os.path.getsize(path)
            start = 0
            if ranges.startswith("bytes=") and not args.no_ranges:
                first = ranges[len("bytes=") :].split("-", 1)[0]
                start = int(first) if first else 0
                if start >= size:
                    self.empty(416, {"Content-Range": f"bytes */{size}"})
                    return
                self.send_response(206)
                self.send_header("Content-Range", f"bytes {start}-{size - 1}/{size}")
            else:
                self.send_response(200)
            if not args.no_ranges:
                self.send_header("Accept-Ranges", "bytes")
            self.send_header("Content-Length", str(size - start))
            self.end_headers()
            if not body:
                return
            chunk = max(1, args.rate // 20) if args.rate else 1 << 16
            with open(path, "rb") as source:
                source.seek(start)
                while data := source.read(chunk):
                    try:
                        self.wfile.write(data)
                        self.wfile.flush()
                    except (BrokenPipeError, ConnectionResetError):
                        return
                    if args.rate:
                        time.sleep(len(data) / args.rate)

    server = http.server.ThreadingHTTPServer(("127.0.0.1", 0), Handler)
    server.daemon_threads = True
    with open(args.port_file + ".tmp", "w", encoding="utf-8") as port_file:
        port_file.write(str(server.server_address[1]))
    os.replace(args.port_file + ".tmp", args.port_file)
    server.serve_forever()


if __name__ == "__main__":
    main()
