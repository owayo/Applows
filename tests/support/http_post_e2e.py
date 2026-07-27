"""http_post / read_stdin / hostname の実 OS E2E.

ローカルのエコーサーバを bind してから被験スクリプト (生成された .bat) を起動し、
サーバが実際に受け取ったリクエストを検証する。macOS (`/bin/sh`) と
Windows (`cmd /c` -> Windows PowerShell 5.1) の両方で同じ検証を行う。

usage: python3 tests/support/http_post_e2e.py <script.bat>
"""

import http.server
import json
import os
import subprocess
import sys
import threading

PORT = 8971
TOKEN = "test-token-value"  # noqa: S105 (テスト用の固定値)
BODY = '{"event":"session.started","cwd":"/tmp/プロジェクト 🚀"}'

received = []


class Handler(http.server.BaseHTTPRequestHandler):
    """POST を記録して 200 を返すだけのハンドラ."""

    def do_POST(self):  # noqa: N802 (BaseHTTPRequestHandler の命名規約)
        """受信したヘッダと body を記録する."""
        length = int(self.headers.get("Content-Length", 0))
        raw = self.rfile.read(length)
        received.append(
            {
                "path": self.path,
                "content_type": self.headers.get("Content-Type"),
                "authorization": self.headers.get("Authorization"),
                "x_host": self.headers.get("X-Host"),
                "injected": self.headers.get("Injected"),
                "body": raw.decode("utf-8"),
            }
        )
        self.send_response(200)
        self.send_header("Content-Length", "2")
        self.end_headers()
        self.wfile.write(b"ok")

    def log_message(self, *args):
        """アクセスログを抑止する."""


def main() -> int:
    """エコーサーバを立てて被験スクリプトを実行し、受信内容を検証する.

    Returns:
        検証に成功したら 0、失敗があれば 1。

    """
    # Windows の Python は既定で cp1252 の stdout になり、body に含む
    # 日本語・絵文字を print した時点で UnicodeEncodeError になる。
    # 検証結果の表示で落ちないよう UTF-8 に揃える。
    sys.stdout.reconfigure(encoding="utf-8", errors="replace")

    script = sys.argv[1]
    server = http.server.HTTPServer(("127.0.0.1", PORT), Handler)
    threading.Thread(target=server.serve_forever, daemon=True).start()

    env = dict(os.environ)
    env["APPLOWS_TEST_URL"] = f"http://127.0.0.1:{PORT}/api/v1/events"
    env["APPLOWS_TEST_TOKEN"] = TOKEN

    argv = ["cmd", "/c", script] if sys.platform == "win32" else ["/bin/sh", script]
    proc = subprocess.run(  # noqa: S603
        argv,
        input=BODY.encode("utf-8"),
        capture_output=True,
        timeout=120,
        check=False,
        env=env,
    )
    server.shutdown()

    out = proc.stdout.decode("utf-8", "replace")
    err = proc.stderr.decode("utf-8", "replace")
    print("--- stdout ---")
    print(out)
    print("--- stderr ---")
    print(err)
    print("--- received ---")
    print(json.dumps(received, ensure_ascii=False, indent=2))

    failures = []
    if proc.returncode != 0:
        failures.append(f"exit code = {proc.returncode}")
    if "rc=0" not in out:
        failures.append("http_post が成功 (rc=0) を返していない")
    # CR/LF を含むヘッダは送信せず 2 を返すこと
    if "bad=2" not in out:
        failures.append("CR/LF を含むヘッダが拒否されていない (bad=2 が無い)")
    if len(received) != 1:
        failures.append(f"受信リクエスト数が 1 でない: {len(received)}")
    if received:
        got = received[0]
        if got["authorization"] != f"Bearer {TOKEN}":
            failures.append(f"Authorization が届いていない: {got['authorization']!r}")
        if got["content_type"] != "application/json":
            failures.append(f"Content-Type が違う: {got['content_type']!r}")
        if got["body"] != BODY:
            failures.append(f"body が一致しない: {got['body']!r}")
        if got["injected"] is not None:
            failures.append("ヘッダインジェクションが通ってしまった")
        if not got["x_host"]:
            failures.append("hostname() が空")

    if failures:
        for f in failures:
            print(f"FAIL: {f}")
        return 1
    print("http_post e2e ok")
    return 0


if __name__ == "__main__":
    sys.exit(main())
