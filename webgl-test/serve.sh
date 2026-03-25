#!/usr/bin/env bash
set -euo pipefail

LOCAL_IP=$(ipconfig getifaddr en0 2>/dev/null || echo "<your-ip>")
echo "==> Serving at http://localhost:8081"
echo "==> On your tablet, open http://$LOCAL_IP:8081"
python3 -c "
import http.server, os

os.chdir('$(dirname "$0")')

class Handler(http.server.SimpleHTTPRequestHandler):
    def end_headers(self):
        self.send_header('Cross-Origin-Opener-Policy', 'same-origin')
        self.send_header('Cross-Origin-Embedder-Policy', 'require-corp')
        self.send_header('Cache-Control', 'no-store')
        super().end_headers()

http.server.HTTPServer(('0.0.0.0', 8081), Handler).serve_forever()
"
