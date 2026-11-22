"""Check that a remote Markdown image loads only after a click in the WebView UI."""

from html import unescape
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
import json
import os
from pathlib import Path
import re
import shutil
import subprocess
import tempfile
import threading

ROOT = Path(__file__).resolve().parents[1]
WEB = ROOT / "src" / "portable"
IMAGE = (ROOT / "assets" / "pane-md.png").read_bytes()


def main():
    chrome = os.environ.get("CHROME_PATH") or shutil.which("chromium") or shutil.which("google-chrome")
    if not chrome:
        candidate = Path(os.environ.get("ProgramFiles", "C:/Program Files")) / "Google/Chrome/Application/chrome.exe"
        chrome = str(candidate) if candidate.is_file() else None
    if not chrome:
        raise SystemExit("Set CHROME_PATH to your Chromium or Chrome executable.")

    page = (WEB / "app.html").read_text(encoding="utf-8")
    page = re.sub(r'<meta http-equiv="Content-Security-Policy"[^>]*>', "", page)
    page = page.replace('<script defer src="/app.js"></script>',
                        '<script>window.ipc = {postMessage() {}};</script><script defer src="/app.js"></script>')

    class Handler(BaseHTTPRequestHandler):
        def do_GET(self):
            if self.path == "/":
                payload, kind = page.encode(), "text/html; charset=utf-8"
            elif self.path == "/remote.png":
                payload, kind = IMAGE, "image/png"
            elif self.path in {"/app.css", "/components.js", "/source-pane.js", "/review-pane.js", "/appearance.js", "/format.js", "/app.js", "/icon.png"}:
                file = (WEB / self.path[1:]) if self.path != "/icon.png" else ROOT / "assets" / "pane-md.png"
                payload = file.read_bytes()
                kind = "text/javascript" if self.path.endswith(".js") else "text/css" if self.path.endswith(".css") else "image/png"
            else:
                self.send_error(404)
                return
            self.send_response(200)
            self.send_header("Content-Type", kind)
            self.send_header("Content-Length", str(len(payload)))
            self.end_headers()
            self.wfile.write(payload)

        def log_message(self, *_):
            pass

    server = ThreadingHTTPServer(("127.0.0.1", 0), Handler)
    threading.Thread(target=server.serve_forever, daemon=True).start()
    port = server.server_port
    url = f"http://127.0.0.1:{port}/remote.png"
    scenario = f"""<script>
window.addEventListener('load', () => {{
  const preview = document.getElementById('preview');
  const url = {json.dumps(url)};
  preview.innerHTML = `<span class="remote-image"><img data-remote-image="${{url}}" hidden><button data-load-remote-image="${{url}}">Load image</button></span>`;
  const image = preview.querySelector('img');
  const button = preview.querySelector('button');
  const output = document.getElementById('result');
  let error = null;
  window.onerror = (message) => {{ error = String(message); }};
  const unloaded = !image.getAttribute('src') && image.hidden && !button.hidden;
  image.addEventListener('load', () => {{
    setTimeout(() => {{ output.textContent = JSON.stringify({{unloaded, loaded: !image.hidden && button.hidden && image.currentSrc === url}}); }}, 0);
  }});
  button.click();
  setTimeout(() => {{ if (!output.textContent) output.textContent = JSON.stringify({{unloaded, loaded: false, src: image.src, complete: image.complete, width: image.naturalWidth, error}}); }}, 1500);
}});
</script><output id="result"></output>"""
    page = page.replace("</body>", scenario + "</body>")

    try:
        with tempfile.TemporaryDirectory(prefix="pane-image-test-") as profile:
            run = subprocess.run(
                [chrome, "--headless", "--disable-gpu", "--no-sandbox", "--disable-dev-shm-usage", f"--user-data-dir={profile}", "--virtual-time-budget=2500", "--dump-dom", f"http://127.0.0.1:{port}/"],
                capture_output=True, encoding="utf-8", errors="replace", timeout=30,
            )
            if run.returncode:
                raise SystemExit(f"Chromium exited {run.returncode}: {run.stderr[-3000:]}")
        match = re.search(r'<output id="result">(.*?)</output>', run.stdout, re.DOTALL)
        if not match:
            raise SystemExit("Browser component test did not return a result.")
        report = json.loads(unescape(match[1]))
        print(json.dumps(report))
        return not all(report.values())
    finally:
        server.shutdown()


if __name__ == "__main__":
    raise SystemExit(main())
