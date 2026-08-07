#!/bin/sh
set -eu

ROOT=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
BIND_ADDR=127.0.0.1
PORT=8082

if [ "${1:-}" = "--global" ]; then
  BIND_ADDR=0.0.0.0
fi

cd "$ROOT"

echo "Serving http://$BIND_ADDR:$PORT"
python3 -c "
import http.server

class Handler(http.server.SimpleHTTPRequestHandler):
    def end_headers(self):
        self.send_header('Cross-Origin-Opener-Policy', 'same-origin')
        self.send_header('Cross-Origin-Embedder-Policy', 'require-corp')
        self.send_header('Cache-Control', 'no-store')
        super().end_headers()

http.server.ThreadingHTTPServer(('$BIND_ADDR', $PORT), Handler).serve_forever()
"
