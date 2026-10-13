"""Loopback-only fault proxy: lose a committed ACK without changing the command."""
import hashlib
import http.client
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
import json
import os
from pathlib import Path
import threading

ROOT = Path(os.environ["LABEL_EVIDENCE"])
CONFIG = json.loads((ROOT / "config.json").read_text())
LOCK = threading.Lock()
SEQUENCE = 0


class Handler(BaseHTTPRequestHandler):
    def log_message(self, *_args):
        pass

    def do_GET(self):
        self.forward()

    def do_POST(self):
        self.forward()

    def forward(self):
        global SEQUENCE
        length = int(self.headers.get("content-length", "0"))
        if length > 1024 * 1024:
            self.send_error(413)
            return
        body = self.rfile.read(length)
        with LOCK:
            SEQUENCE += 1
            number = SEQUENCE
            backend = CONFIG["backends"][number % 2]
            drop = self.path == "/events" and (ROOT / "drop-next-ack").exists()
            if drop:
                (ROOT / "drop-next-ack").unlink()
        event = json.loads(body) if self.path == "/events" else {}
        entry = {"sequence": number, "backend": backend, "path": self.path,
                 "event": event.get("id"), "body_sha256": hashlib.sha256(body).hexdigest(),
                 "auth_sha256": hashlib.sha256(self.headers.get("authorization", "").encode()).hexdigest()}
        conn = http.client.HTTPConnection("127.0.0.1", backend, timeout=30)
        try:
            headers = {key: value for key, value in self.headers.items()
                       if key.lower() not in ("content-length", "connection", "accept-encoding")}
            conn.request(self.command, self.path, body, headers)
            response = conn.getresponse()
            data = response.read(2 * 1024 * 1024 + 1)
            if len(data) > 2 * 1024 * 1024:
                raise ValueError("response exceeds fixture budget")
            entry["status"] = response.status
            if self.path == "/events":
                entry["ack"] = json.loads(data)
            entry["dropped"] = drop
            with LOCK:
                with (ROOT / "PROXY.jsonl").open("a") as log:
                    log.write(json.dumps(entry) + "\n")
            if drop:
                self.close_connection = True
                self.connection.shutdown(2)
                self.connection.close()
                return
            self.send_response(response.status)
            for key, value in response.getheaders():
                if key.lower() not in ("transfer-encoding", "content-length", "connection", "content-encoding"):
                    self.send_header(key, value)
            self.send_header("Content-Length", str(len(data)))
            self.end_headers()
            self.wfile.write(data)
        except Exception as error:
            with LOCK:
                with (ROOT / "PROXY_ERRORS.log").open("a") as log:
                    log.write(str(error) + "\n")
            self.close_connection = True
        finally:
            conn.close()


ThreadingHTTPServer(("127.0.0.1", CONFIG["proxy"]), Handler).serve_forever()
