import http.server
import os
import sys


class Handler(http.server.SimpleHTTPRequestHandler):
    def end_headers(self):
        self.send_header("Cache-Control", "no-store")
        super().end_headers()


def main():
    if len(sys.argv) != 4:
        raise SystemExit("usage: server.py DIST BIND_ADDR PORT")
    dist, bind_addr, port = sys.argv[1], sys.argv[2], int(sys.argv[3])
    os.chdir(dist)
    http.server.ThreadingHTTPServer((bind_addr, port), Handler).serve_forever()


if __name__ == "__main__":
    main()

