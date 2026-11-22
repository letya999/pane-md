"""Exercise the Linux WebKit window using xdotool and ImageMagick on X11."""

import argparse
from contextlib import contextmanager
import json
import os
from pathlib import Path
import re
import select
import subprocess
import tempfile
import time


def xdo(*args):
    return subprocess.run(["xdotool", *args], text=True, capture_output=True, timeout=5)


def pixel_at(window, x, y):
    return subprocess.check_output([
        "import", "-window", window, "-crop", f"1x1+{x}+{y}", "-depth", "8", "txt:-",
    ], timeout=5)


def mode_is(window, family):
    # The selected segment is the only blue fill in the document toolbar.
    pixels = subprocess.check_output([
        "import", "-window", window, "-crop", "x1+0+86", "-depth", "8", "rgb:-",
    ], timeout=5)
    selected = [pixels[i:i + 3] == bytes.fromhex("cfe3fc") for i in range(0, len(pixels), 3)]
    length = max(map(len, "".join("1" if pixel else "0" for pixel in selected).split("0")))
    return length >= 24 and (length > 60) == (family == "rich")


def until(check, description, seconds=12):
    end = time.monotonic() + seconds
    while time.monotonic() < end:
        result = check()
        if result:
            return result
        time.sleep(.05)
    raise AssertionError(description)


def check_workspace(exe, folder, export=None):
    settings = folder / "workspace-config" / "pane-md" / "settings.json"
    settings.parent.mkdir(parents=True)
    settings.write_text('{"english":false,"theme":"light"}', encoding="utf-8")
    env = dict(os.environ, XDG_CONFIG_HOME=str(settings.parent.parent))
    fixture = folder / "README.md"
    text = """# pane-md

Локальный блокнот для чтения, правки и ревью Markdown на Windows.

## Сборка и запуск

Установите Rust и Visual Studio Build Tools с Windows SDK.

```powershell
cargo build --release --locked
.\\target\\release\\pane-md.exe README.md
```

Откройте файл через Ctrl+O или перетащите его в окно.

## Ревью

Выделите текст, чтобы оставить комментарий.
"""
    for quote, heading, prefix, suffix, block, comment in [
        ("Windows SDK", "Сборка и запуск", "Tools с ", ".", "paragraph", "Уточнить версию SDK."),
        ("Ctrl+O", "Сборка и запуск", "через ", " или", "paragraph", "Добавить сочетание для открытия папки."),
        ("Ревью", "Ревью", "", "", "heading", "Пояснить, где хранятся комментарии."),
    ]:
        payload = dict(v=1, id=f"review-{len(text)}", quote=quote,
                       anchor=dict(heading=heading, block=block, prefix=prefix, suffix=suffix),
                       text=comment, status="open")
        line = next(row for row in text.splitlines() if quote in row and not row.startswith("<!--"))
        record = "<!-- pane-md:review " + json.dumps(payload, ensure_ascii=False, separators=(",", ":")) + " -->"
        text = text.replace(line + "\n", line + "\n\n" + record + "\n", 1)
    fixture.write_text(text, encoding="utf-8")

    with editor_window(exe, fixture, env, wait=False) as window:
        def click(x, y):
            assert xdo("mousemove", "--window", window, str(x), str(y), "click", "1").returncode == 0

        def screenshot(name):
            if export:
                export.mkdir(parents=True, exist_ok=True)
                subprocess.run(["import", "-window", window, str(export / f"{name}.png")], check=True, timeout=5)

        def color(x, y, value):
            return value.encode() in pixel_at(window, x, y)

        until(lambda: mode_is(window, "rich"), "WYSIWYG toolbar did not paint")
        xdo("key", "ctrl+n")
        until(lambda: "1 — pane-md" in xdo("getwindowname", window).stdout, "new tab did not appear")
        xdo("key", "ctrl+Tab")
        until(lambda: "README.md — pane-md" in xdo("getwindowname", window).stdout, "file tab did not activate")
        click(24, 98)
        until(lambda: color(100, 500, "#F1F0EE"), "outline did not expand")
        click(930, 154)
        screenshot("wysiwyg")

        click(358, 98)
        until(lambda: mode_is(window, "source"), "MD selector did not preserve comments mode")
        assert color(250, 500, "#F1F0EE"), "source number strip is missing"
        screenshot("markdown")

        click(197, 98)
        until(lambda: color(100, 500, "#FBFAF8"), "collapsed outline did not release its column")
        click(1135, 98)
        until(lambda: color(1135, 500, "#FBFAF8"), "collapsed comments did not release its column")
        click(1135, 98)
        until(lambda: color(1135, 500, "#F1F0EE"), "comments did not expand")
        click(930, 154)
        xdo("key", "ctrl+alt+e")
        until(lambda: color(889, 180, "#125EBF"), "the selected review did not open inline")
        xdo("key", "ctrl+a")
        with clipboard_text("Уточнить версию SDK для Windows 11."):
            xdo("key", "ctrl+v")
            time.sleep(.2)
        assert fixture.read_text(encoding="utf-8") == text, "typing committed the inline draft"
        screenshot("comment-edit")
        click(1090, 295)

        def saved():
            xdo("key", "ctrl+s")
            return "Уточнить версию SDK для Windows 11." in fixture.read_text(encoding="utf-8")

        until(saved, "the inline comment edit did not save")
        result = subprocess.check_output([exe, "--list-comments", str(fixture)], timeout=5)
        assert len(json.loads(result)["comments"]) == 3, "editing one review affected another card"
        click(755, 98)
        xdo("key", "--delay", "80", "Home", "Return")
        until(lambda: color(1135, 500, "#FBFAF8"), "read-only mode did not hide comments")
        assert mode_is(window, "source"), "changing interaction mode switched the editor view"
        click(1040, 98)
        xdo("key", "--delay", "80", "Home", "Down", "Return")
        until(lambda: color(1135, 500, "#F1F0EE"), "comments mode did not restore the panel")
        click(24, 98)
        until(lambda: color(100, 500, "#F1F0EE"), "outline did not reopen")
        xdo("windowsize", window, "610", "720")
        until(lambda: color(100, 500, "#FBFAF8"), "compact window did not make room for source")
        click(22, 98)
        until(lambda: color(100, 500, "#F1F0EE"), "compact window did not restore the outline")


@contextmanager
def editor_window(exe, fixture, env=None, wait=True):
    argv = [exe, "--source", "--wait", str(fixture)] if wait else [exe, "--gui", str(fixture)]
    with subprocess.Popen(argv, stdout=subprocess.DEVNULL, env=env) as process:
        try:
            title = fixture.name + " — pane-md" if wait else "^pane-md$"

            def window():
                response = xdo("search", "--onlyvisible", "--pid", str(process.pid), "--name", title)
                return response.stdout.splitlines()[0] if response.stdout else None

            wid = until(window, "editor did not open the document")
            assert process.poll() is None
            assert xdo("windowfocus", "--sync", wid).returncode == 0
            if wait:
                def tab_is_painted():
                    colors = subprocess.check_output([
                        "import", "-window", wid, "-crop", "100x24+15+45", "-format", "%k", "info:",
                    ], timeout=5)
                    return int(colors) > 1

                until(tab_is_painted, "the document tab did not paint")
            yield wid
            if wait:
                xdo("key", "ctrl+w")
                exit_code = process.wait(timeout=8)
                assert exit_code == 0, f"{fixture.name} closed with exit code {exit_code}"
        finally:
            if process.poll() is None:
                process.terminate()
                process.wait(timeout=5)


def find_text(needle):
    xdo("key", "ctrl+f")
    time.sleep(.18)
    xdo("type", needle)
    xdo("key", "Escape")
    time.sleep(.12)


def check_window(exe, fixture, original):
    settings = fixture.parent / "source-config" / "pane-md" / "settings.json"
    settings.parent.mkdir(parents=True)
    settings.write_text('{"english":true,"theme":"light"}', encoding="utf-8")
    env = dict(os.environ, XDG_CONFIG_HOME=str(settings.parent.parent))
    with editor_window(exe, fixture, env) as window:
        xdo("key", "ctrl+1")
        until(lambda: mode_is(window, "rich"), "WYSIWYG did not enter reading mode")
        xdo("type", "Read-only input must be ignored")
        xdo("key", "ctrl+4")

        def source_ready():
            xdo("key", "ctrl+a", "ctrl+c")
            try:
                clipboard = subprocess.run(["xclip", "-selection", "clipboard", "-o"],
                                           capture_output=True, timeout=2).stdout
            except subprocess.TimeoutExpired:
                return False
            return clipboard.replace(b"\r\n", b"\n") == original[3:].replace(b"\r\n", b"\n")

        with clipboard_text("Waiting for Markdown source"):
            until(source_ready, "Markdown source did not regain focus")
        find_text("formatting")
        xdo("key", "ctrl+End")
        xdo("type", "Saved from Linux")
        xdo("key", "ctrl+s")
        expected = original + b"Saved from Linux"
        until(lambda: fixture.read_bytes() == expected, "source edit was not saved verbatim")
        time.sleep(.15)
        xdo("type", "!")
        xdo("key", "ctrl+s")
        expected += b"!"
        until(lambda: fixture.read_bytes() == expected, "saving displaced the source caret")
    assert fixture.read_bytes() == expected


def check_large_source(exe, fixture):
    needle = "unique middle checkpoint"
    original = ("\ufeff# Большой исходник 😀\r\n\r\n" +
                "".join(f"Line {index:05d} with text.\r\n" for index in range(15000)) +
                needle + "\r\n" +
                "".join(f"Line {index:05d} with text.\r\n" for index in range(15000, 30000))).encode()
    fixture.write_bytes(original)
    replacement = "готово 😀\ncontinued"

    with editor_window(exe, fixture) as window:
        until(lambda: mode_is(window, "source"), "large source did not open")
        xdo("key", "ctrl+f")
        time.sleep(.3)
        with clipboard_text(needle):
            xdo("key", "ctrl+v")
            time.sleep(.5)
            xdo("key", "Escape")
        time.sleep(.4)
        with clipboard_text(replacement):
            xdo("key", "ctrl+v")
            time.sleep(.7)
        xdo("key", "ctrl+s")
        expected = original.replace(needle.encode(), replacement.replace("\n", "\r\n").encode())
        until(lambda: fixture.read_bytes() == expected, "the large source edit did not preserve the rest of the file")


def check_mcp_open(exe, folder):
    fixture = folder / "MCP — заметка.md"
    original = "\ufeff# Окно из MCP\r\n\r\nДокумент открыт для просмотра.\r\n".encode()
    fixture.write_bytes(original)
    settings = folder / "mcp-config" / "pane-md" / "settings.json"
    settings.parent.mkdir(parents=True)
    settings.write_text('{"english":true,"theme":"light"}', encoding="utf-8")
    env = dict(os.environ, XDG_CONFIG_HOME=str(settings.parent.parent))
    title = f"^{re.escape(fixture.name)} — pane-md$"

    def window():
        found = xdo("search", "--onlyvisible", "--name", title)
        return found.stdout.splitlines()[0] if found.stdout else None

    def close(wid):
        assert xdo("windowfocus", "--sync", wid).returncode == 0
        assert xdo("mousemove", "--window", wid, "55", "16", "click", "1").returncode == 0
        assert xdo("mousemove", "--window", wid, "67", "218", "click", "1").returncode == 0

    with subprocess.Popen([exe, "--mcp"], cwd=folder, env=env, stdin=subprocess.PIPE,
                          stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True,
                          encoding="utf-8") as server:
        sequence = 0

        def request(method, params, *, notice=False):
            nonlocal sequence
            message = {"jsonrpc": "2.0", "method": method, "params": params}
            if not notice:
                sequence += 1
                message["id"] = sequence
            server.stdin.write(json.dumps(message) + "\n")
            server.stdin.flush()
            if notice:
                return None
            assert select.select([server.stdout], [], [], 10)[0], f"MCP did not reply to {method}"
            response = json.loads(server.stdout.readline())
            assert response["id"] == sequence, response
            return response

        def open_document(path):
            return request("tools/call", {"name": "open_document", "arguments": {"path": path}})

        def open_window():
            response = open_document(fixture.name)["result"]["structuredContent"]
            assert Path(response["path"]) == fixture and response["status"] == "launched"
            wid = until(window, "MCP did not open the requested window")
            until(lambda: mode_is(wid, "rich"), "the MCP window did not render WYSIWYG")
            pid = int(xdo("getwindowpid", wid).stdout)
            assert pid != server.pid
            return wid, pid

        try:
            init = request("initialize", {"protocolVersion": "2025-11-25", "capabilities": {},
                                          "clientInfo": {"name": "linux-smoke", "version": "1"}})
            assert "result" in init, init
            request("notifications/initialized", {}, notice=True)
            tools = request("tools/list", {})["result"]["tools"]
            assert "open_document" in {tool["name"] for tool in tools}
            assert "error" in open_document("missing.md")

            first, pid = open_window()
            close(first)
            until(lambda: not Path(f"/proc/{pid}").exists(), "MCP did not reap the closed editor")
            assert server.poll() is None

            second, _ = open_window()
            server.stdin.close()
            server.stdin = None
            remaining, diagnostics = server.communicate(timeout=8)
            assert server.returncode == 0 and not remaining
            assert "missing.md" in diagnostics
            assert window() == second, "the editor closed with its MCP server"
            assert fixture.read_bytes() == original
        finally:
            if wid := window():
                close(wid)
                until(lambda: not window(), "the MCP window did not close")


def check_reviews(exe, fixture, original):
    def wait_for_record(check, explanation):
        time.sleep(.25)

        def read():
            xdo("key", "ctrl+s")
            result = subprocess.run(
                [exe, "--list-comments", str(fixture)], check=True, capture_output=True, timeout=4,
            )
            report = json.loads(result.stdout)
            comments = report["comments"]
            if check(comments):
                assert report["diagnostics"] == []
                return (comments,)

        return until(read, explanation)[0]

    with editor_window(exe, fixture) as window:
        def wait_draft():
            def focused():
                pixels = subprocess.check_output([
                    "import", "-window", window, "-crop", "7x74+887+148",
                    "-depth", "8", "-format", "%c", "histogram:info:",
                ], timeout=5)
                return b"#125EBF" in pixels or b"#8BC5FF" in pixels

            until(focused, "the focused comment draft did not appear")

        find_text("First annotated text.")
        xdo("key", "ctrl+alt+m")
        wait_draft()
        xdo("type", "Initial review")
        xdo("key", "Return")
        xdo("type", "Second line")
        xdo("key", "ctrl+Return")
        first, = wait_for_record(lambda records: len(records) == 1, "source comment was not saved")
        assert first["quote"] == "First annotated text."
        assert first["text"] == "Initial review\nSecond line"
        assert first["state"] == "attached"

        xdo("key", "ctrl+alt+e")
        wait_draft()
        xdo("key", "ctrl+a")
        xdo("type", "Revised")
        xdo("key", "ctrl+Return")
        wait_for_record(lambda records: records[0]["text"] == "Revised", "comment edits were not saved")
        xdo("key", "ctrl+alt+r")
        wait_for_record(lambda records: records[0]["status"] == "resolved", "comment did not resolve")
        xdo("key", "ctrl+alt+r")
        wait_for_record(lambda records: records[0]["status"] == "open", "comment did not reopen")

        find_text("Second paragraph.")
        xdo("key", "ctrl+alt+l")
        moved, = wait_for_record(lambda records: records[0]["quote"] == "Second paragraph.",
                                 "comment did not reattach")
        assert moved["id"] == first["id"]
        assert moved["state"] == "attached"
        xdo("key", "ctrl+alt+d")
        time.sleep(.1)
        xdo("key", "shift+Tab", "Return")
        wait_for_record(lambda records: not records, "confirmed deletion did not save")

        xdo("key", "ctrl+3")
        time.sleep(.25)
        find_text("First annotated text.")
        xdo("key", "ctrl+alt+m")
        wait_draft()
        xdo("type", "In WYSIWYG")
        xdo("key", "ctrl+Return")
        visual, = wait_for_record(lambda records: len(records) == 1, "visual comment was not saved")
        assert visual["quote"] == "First annotated text."
        assert visual["text"] == "In WYSIWYG"
        assert visual["state"] == "attached"
    saved = fixture.read_bytes()
    assert saved.startswith(b"\xef\xbb\xbf")
    assert saved.count(b"\r\n") == saved.count(b"\n")
    for line in original.split(b"\r\n"):
        assert line in saved.split(b"\r\n")


def check_mermaid(exe, fixture, screenshot, export):
    diagram = (
        "```mermaid\nflowchart LR\n"
        "    Draft[Draft] --> Check[Check links] --> Publish[Publish]\n"
        "    style Draft fill:#b92975,color:#fff\n```"
    )
    original = f"# Diagram previews\n\n{diagram}\n\n## Source fallback\n\n```mermaid\ninvalid input\n```\n"
    fixture.write_text(original, encoding="utf-8")
    with editor_window(exe, fixture) as wid:
        xdo("key", "ctrl+3")

        def capture():
            subprocess.run(["import", "-window", wid, str(screenshot)], check=True, timeout=5)
            # This color can come only from the rendered node in the fixture.
            mask = subprocess.check_output([
                "convert", "-quiet", str(screenshot), "-alpha", "off", "-fill", "black", "+opaque",
                "#b92975", "-fill", "white", "-opaque", "#b92975",
                "-format", "%[fx:mean] %@ ", "info:",
            ], text=True, timeout=5)
            coverage, box = mask.split()
            return box if float(coverage) > .001 else ""

        node = until(capture, "Mermaid PNG was not displayed", seconds=20)
        if export:
            export.write_bytes(screenshot.read_bytes())
        size, left, top = node.split("+")
        width, height = map(int, size.split("x"))
        xdo("mousemove", "--window", wid, str(int(left) + width // 2), str(int(top) + height // 2),
            "click", "1", "key", "Tab", "Return")
        time.sleep(.3)
        xdo("type", "Diagram replaced")
        xdo("key", "ctrl+s")
        expected = original.replace(diagram, "Diagram replaced")
        until(lambda: fixture.read_text(encoding="utf-8") == expected,
              "the source action did not select the whole Mermaid block")


def check_appearance(exe, fixture, export):
    # Use a private configuration and the editor's default 1160x780 X11 window.
    config = fixture.parent / "appearance-config"
    settings = config / "pane-md" / "settings.json"
    settings.parent.mkdir(parents=True)
    settings.write_text('{"english":true,"theme":"light"}', encoding="utf-8")
    env = dict(os.environ, XDG_CONFIG_HOME=str(config))
    original = "# Appearance\n\nA paragraph with a [link](#appearance) and **bold**.\n"
    fixture.write_text(original, encoding="utf-8")

    def click(window, x, y):
        assert xdo("mousemove", "--window", window, str(x), str(y), "click", "1").returncode == 0

    def enter(window, x, y, text):
        click(window, x, y)
        xdo("key", "ctrl+a")
        xdo("type", "--delay", "45", text)

    def choose(window, x, y, *keys):
        click(window, x, y)
        time.sleep(.2)  # WebKit opens the native popup asynchronously.
        xdo("key", "--delay", "90", *keys, "Return")
        time.sleep(.15)

    def open_dialog(window):
        click(window, 273, 16)
        click(window, 280, 49)
        time.sleep(.75)

    def preferences():
        return json.loads(settings.read_text(encoding="utf-8"))

    def painted(window, color):
        return color.upper().encode() in pixel_at(window, 800, 700)

    with editor_window(exe, fixture, env) as window:
        until(lambda: mode_is(window, "source"), "Markdown editor did not become active")
        xdo("key", "ctrl+End")
        xdo("type", "Pending edit")
        open_dialog(window)
        choose(window, 704, 251, "Home", "Down")
        enter(window, 751, 250, "20")
        choose(window, 705, 321, "Home", "Down")
        enter(window, 751, 321, "17.5")
        enter(window, 428, 472, "#f7e6cc")
        enter(window, 584, 472, "#2c1e15")
        enter(window, 739, 472, "#2545a8")
        choose(window, 804, 404, "Down")
        enter(window, 428, 472, "#16263d")
        choose(window, 804, 404, "Up")
        if export:
            subprocess.run(["import", "-window", window, str(export)], check=True, timeout=5)
        click(window, 786, 595)
        until(lambda: "appearance" in preferences(), "appearance settings were not saved")
        appearance = preferences()["appearance"]
        assert appearance["rich"]["family"]
        assert appearance["source"]["family"]
        assert appearance["rich"]["half_points"] == 40
        assert appearance["source"]["half_points"] == 35
        assert appearance["light"] == {"paper": 0xf7e6cc, "ink": 0x2c1e15, "link": 0x2545a8}
        assert appearance["dark"]["paper"] == 0x16263d
        until(lambda: painted(window, "#f7e6cc"), "Markdown background did not update")
        xdo("key", "ctrl+1")
        until(lambda: mode_is(window, "rich"), "WYSIWYG did not become active")
        until(lambda: painted(window, "#f7e6cc"), "WYSIWYG background did not update")
        xdo("key", "ctrl+s")
        until(lambda: fixture.read_text(encoding="utf-8") == original + "Pending edit",
              "applying appearance lost a Markdown edit")

    with editor_window(exe, fixture, env) as window:
        until(lambda: painted(window, "#f7e6cc"), "document appearance did not survive restart")
        open_dialog(window)
        enter(window, 428, 472, "#ffffff")
        xdo("key", "Escape")
        assert preferences()["appearance"] == appearance, "cancel changed stored appearance"
        click(window, 273, 16)
        click(window, 280, 102)
        until(lambda: preferences()["theme"] == "dark", "could not switch to the dark theme")
        until(lambda: painted(window, "#16263d"), "dark theme lost its own background")
        open_dialog(window)
        click(window, 425, 511)  # Reset theme colors.
        click(window, 786, 595)
        until(lambda: preferences()["appearance"]["dark"]["paper"] is None,
              "dark overrides were not cleared")
        assert preferences()["appearance"]["light"] == appearance["light"]
        until(lambda: painted(window, "#1e2125"), "dark defaults did not return")

        open_dialog(window)
        click(window, 428, 595)  # Reset fonts and colors.
        click(window, 786, 595)
        until(lambda: preferences()["appearance"]["rich"]["family"] == "",
              "editor defaults were not restored")
        restored = preferences()["appearance"]
        assert restored["rich"]["half_points"] == 24
        assert restored["source"] == {"family": "", "half_points": 23}
        assert all(color is None for color in restored["light"].values())


def check_formatting(exe, fixture, export):
    config = fixture.parent / "format-config"
    settings = config / "pane-md" / "settings.json"
    settings.parent.mkdir(parents=True)
    settings.write_text('{"english":true,"theme":"light"}', encoding="utf-8")
    env = dict(os.environ, XDG_CONFIG_HOME=str(config))
    fixture.write_text(
        "# Plan\n\nAlpha text and a word.\n\nTask item\n\nSection title\n\n"
        "| A | B |\n| --- | --- |\n| 1 | Second cell |\n",
        encoding="utf-8",
    )

    def find(text):
        find_text(text)
        time.sleep(.18)

    def saved(expected):
        time.sleep(.35)
        xdo("key", "ctrl+s")
        until(lambda: expected in fixture.read_text(encoding="utf-8"),
              f"visual edit did not save Markdown {expected!r}")

    with editor_window(exe, fixture, env) as window:
        xdo("key", "ctrl+3")
        time.sleep(.45)
        find("Alpha")
        xdo("key", "ctrl+alt+m")
        time.sleep(.4)
        xdo("type", "Keep this comment")
        xdo("key", "ctrl+Return")
        saved("Keep this comment")

        find("Alpha")
        xdo("key", "ctrl+b")
        saved("**Alpha**")
        report = subprocess.check_output([exe, "--list-comments", str(fixture)])
        record, = json.loads(report)["comments"]
        assert record["quote"] == "Alpha"
        assert record["state"] == "attached"
        find("Alpha")
        xdo("key", "ctrl+b")
        saved("Alpha text")
        xdo("key", "ctrl+z")
        saved("**Alpha**")

        find("word")
        xdo("key", "ctrl+k")
        time.sleep(.35)
        xdo("type", "#plan")
        xdo("key", "Return")
        saved("[word](#plan)")

        find("Task item")
        xdo("key", "ctrl+shift+l")
        time.sleep(.3)
        xdo("key", "End", "Return")
        time.sleep(.4)
        xdo("type", "Second task")
        saved("- [ ] Task item\n- [ ] Second task")

        find("Section title")
        xdo("key", "ctrl+alt+2")
        saved("## Section title")

        find("Second cell")
        for x, y in [(216, 16), (225, 215)]:
            xdo("mousemove", "--window", window, str(x), str(y), "click", "1")
            time.sleep(.14)
        if export:
            subprocess.run(["import", "-window", window, str(export)], check=True, timeout=5)
        xdo("mousemove", "--window", window, "225", "269", "click", "1")
        saved("| 1 | Second cell |\n|   |   |")

    fixture.write_text("", encoding="utf-8")
    with editor_window(exe, fixture, env) as window:
        xdo("key", "ctrl+3")
        time.sleep(.35)
        xdo("mousemove", "--window", window, "73", "165", "click", "1")
        xdo("type", "First paragraph")
        xdo("key", "Return")
        time.sleep(.4)
        xdo("type", "Second paragraph")
        saved("First paragraph\n\nSecond paragraph")


@contextmanager
def clipboard_text(text):
    payload = text.encode("utf-8")
    command = ["xclip", "-selection", "clipboard"]
    with subprocess.Popen([*command, "-quiet", "-i"], stdin=subprocess.PIPE,
                          stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL) as process:
        try:
            process.stdin.write(payload)
            process.stdin.close()
            until(lambda: subprocess.run([*command, "-o"], capture_output=True, timeout=2).stdout == payload,
                  "the clipboard did not accept the test fixture", seconds=5)
            yield
        finally:
            if process.poll() is None:
                process.terminate()


def check_paste(exe, fixture, export):
    original = "# Clipboard\r\n\r\nKeep old text.\r\n\r\n- [x] Keep **task**.\r\n"
    fixture.write_bytes(original.encode("utf-8"))
    with editor_window(exe, fixture) as window:
        def save(expected):
            time.sleep(.25)
            xdo("key", "ctrl+s")
            until(lambda: fixture.read_bytes() == expected.encode("utf-8"),
                  "pasted text did not round-trip through Markdown")

        def paste(selection, text, expected):
            find_text(selection)
            with clipboard_text(text):
                xdo("key", "ctrl+v")
                save(expected)

        xdo("key", "ctrl+3")
        time.sleep(.45)
        changed = original.replace("old", "First Ж😀<br>\\- literal\r\n\r\nSecond")
        paste("old", "First Ж😀\n- literal\n\nSecond", changed)
        xdo("key", "ctrl+z")
        save(original)
        xdo("key", "ctrl+y")
        save(changed)
        changed_task = changed.replace("**task**", "**task one<br>task two**")
        paste("task", "task one\ntask two", changed_task)
        if export:
            subprocess.run(["import", "-window", window, str(export)], check=True, timeout=5)


def check_visual_replacement(exe, fixture, export):
    settings = fixture.parent / "visual-replace-config" / "pane-md" / "settings.json"
    settings.parent.mkdir(parents=True)
    settings.write_text('{"english":true,"theme":"light"}', encoding="utf-8")
    env = dict(os.environ, XDG_CONFIG_HOME=str(settings.parent.parent))

    def comment(identifier, quote, prefix, suffix, text):
        payload = dict(v=1, id=identifier, quote=quote, text=text, status="open",
                       anchor=dict(block="paragraph", prefix=prefix, suffix=suffix))
        return "<!-- pane-md:review " + json.dumps(payload, separators=(",", ":")) + " -->"

    kept = comment("kept", "bold", "Keep ", " old", "Keep the emphasis.")
    deleted = comment("removed", "Remove", "", " tail", "Review the old wording.")
    original = f"\ufeffKeep **bold** old\r\n\r\n{kept}\r\n\r\n{deleted}\r\n\r\nRemove tail\r\n\r\nAfter.\r\n".encode()
    fixture.write_bytes(original)

    with editor_window(exe, fixture, env) as window:
        xdo("key", "ctrl+3")
        until(lambda: mode_is(window, "rich"), "WYSIWYG did not activate")

        def select(reverse=False):
            start, end = ("130", "187"), ("113", "225")
            if reverse:
                start, end = end, start
            xdo("mousemove", "--window", window, *start, "mousedown", "1")
            time.sleep(.12)
            xdo("mousemove", "--window", window, *end)
            time.sleep(.18)
            xdo("mouseup", "1")

        def clipboard_selection():
            return subprocess.run(["xclip", "-selection", "clipboard", "-o"], capture_output=True,
                                  timeout=2).stdout.strip().split() == [b"old", b"Remove"]

        def save(expected):
            def written():
                xdo("key", "ctrl+s")
                return fixture.read_bytes() == expected

            until(written, "the cross-paragraph change did not save")
            time.sleep(.2)

        def expected(prefix):
            return f"\ufeff{prefix}\r\n\r\n{kept}\r\n\r\n{deleted}\r\n\r\nAfter.\r\n".encode()

        select()
        with clipboard_text("Waiting for selection"):
            xdo("key", "ctrl+c")
            until(clipboard_selection, "a mouse drag did not select both editable paragraphs")
        xdo("type", "X")
        save(expected("Keep **bold** Xtail"))
        report = json.loads(subprocess.check_output([exe, "--list-comments", str(fixture)]))
        assert [(record["id"], record["state"]) for record in report["comments"]] == [
            ("kept", "attached"), ("removed", "missing"),
        ]
        if export:
            subprocess.run(["import", "-window", window, str(export)], check=True, timeout=5)
        xdo("key", "ctrl+z")
        save(original)

        select(reverse=True)
        xdo("key", "Delete")
        save(expected("Keep **bold** tail"))
        xdo("key", "ctrl+z")
        save(original)

        select()
        with clipboard_text("Waiting for cut"):
            xdo("key", "ctrl+x")
            until(clipboard_selection, "cut did not copy the selected paragraphs")
        save(expected("Keep **bold** tail"))
        xdo("key", "ctrl+z")
        save(original)

        select()
        with clipboard_text("new\nline"):
            xdo("key", "ctrl+v")
            save(expected("Keep **bold** new<br>linetail"))
        xdo("key", "ctrl+z")
        save(original)

        select()
        assert xdo("key", "Right").returncode == 0
        xdo("type", "+")
        save(original.replace(b"Remove tail", b"Remove +tail"))
        xdo("key", "ctrl+z")
        save(original)


def check_structured_replacement(exe, fixture, export):
    env = dict(os.environ, XDG_CONFIG_HOME=str(fixture.parent / "structured-edit-config"))
    original = (
        b"- [x] Keep **bold** old\r\n  - [ ] Nested\r\n- [ ] Remove tail\r\n- [ ] After\r\n"
        b"\r\n| A | B |\r\n| :--- | ---: |\r\n| First | Second |\r\n"
    )
    fixture.write_bytes(original)
    with editor_window(exe, fixture, env) as window:
        xdo("key", "ctrl+3")
        until(lambda: mode_is(window, "rich"), "WYSIWYG did not activate")

        def drag(start, end):
            xdo("mousemove", "--window", window, *start, "mousedown", "1")
            time.sleep(.12)
            xdo("mousemove", "--window", window, *end)
            time.sleep(.18)
            xdo("mouseup", "1")

        def save(expected):
            def written():
                xdo("key", "ctrl+s")
                return fixture.read_bytes() == expected

            until(written, "the structured change did not save")
            time.sleep(.2)

        drag(("169", "177"), ("152", "231"))
        xdo("type", "X")
        joined = original.replace(b"old\r\n  - [ ] Nested\r\n- [ ] Remove ", b"X")
        save(joined)

        drag(("65", "271"), ("792", "271"))
        with clipboard_text("two|cells"):
            xdo("key", "ctrl+v")
            save(joined.replace(b"| First | Second |", b"| Fitwo\\|cells | ond |"))
        if export:
            subprocess.run(["import", "-window", window, str(export)], check=True, timeout=5)
        xdo("key", "ctrl+z")
        save(joined)

        drag(("792", "271"), ("65", "271"))
        with clipboard_text("Waiting for cut"):
            xdo("key", "ctrl+x")
            copied = subprocess.run(["xclip", "-selection", "clipboard", "-o"], capture_output=True,
                                    timeout=2)
            assert copied.stdout.split() == [b"rst", b"Sec"], copied.stdout
        save(joined.replace(b"| First | Second |", b"| Fi | ond |"))
        xdo("key", "ctrl+z")
        save(joined)
        xdo("key", "ctrl+z")
        save(original)


def check_sparse_tables(exe, fixture, export):
    config = fixture.parent / "sparse-table-config" / "pane-md" / "settings.json"
    config.parent.mkdir(parents=True)
    config.write_text('{"english":true,"theme":"light"}', encoding="utf-8")
    env = dict(os.environ, XDG_CONFIG_HOME=str(config.parent.parent))
    original = b"\xef\xbb\xbf| A | B | C |\r\n| --- | ---: | :---: |\r\n| x |\r\nOne | Two  \r\n\r\nAfter\r\n"
    fixture.write_bytes(original)
    with editor_window(exe, fixture, env) as window:
        xdo("key", "ctrl+3")
        until(lambda: mode_is(window, "rich"), "WYSIWYG did not activate")

        def click(x, y):
            xdo("mousemove", "--window", window, str(x), str(y), "click", "1")
            time.sleep(.16)

        def save(expected):
            def written():
                xdo("key", "ctrl+s")
                return fixture.read_bytes() == expected

            until(written, "the table edit did not save")
            time.sleep(.2)

        click(725, 215)
        xdo("type", "fill|cell")
        save(original.replace(b"| x |", b"| x |   | fill\\|cell |"))
        xdo("key", "ctrl+z")
        save(original)

        click(720, 249)
        with clipboard_text("pasted|cell"):
            xdo("key", "ctrl+v")
            pasted = original.replace(b"One | Two", b"One | Two | pasted\\|cell")
            save(pasted)

        click(720, 215)
        for x, y in [(216, 16), (225, 215), (225, 326)]:
            click(x, y)
        added = (pasted.replace(b"| A | B | C |", b"| A | B | C |   |")
                       .replace(b"| :---: |", b"| :---: | --- |")
                       .replace(b"pasted\\|cell", b"pasted\\|cell|   |"))
        save(added)
        if export:
            subprocess.run(["import", "-window", window, str(export)], check=True, timeout=5)
        xdo("key", "ctrl+z")
        save(pasted)
        xdo("key", "ctrl+z")
        save(original)


def check_table_keyboard(exe, fixture, export):
    env = dict(os.environ, XDG_CONFIG_HOME=str(fixture.parent / "table-keyboard-config"))
    original = b"| A | B | C |\r\n| --- | --- | --- |\r\n| x |\r\n\r\nAfter.\r\n"
    fixture.write_bytes(original)
    with editor_window(exe, fixture, env) as window:
        xdo("key", "ctrl+3")
        until(lambda: mode_is(window, "rich"), "WYSIWYG did not activate")

        def save(expected):
            def written():
                xdo("key", "ctrl+s")
                return fixture.read_bytes() == expected

            until(written, "the keyboard table edit did not save")
            time.sleep(.2)

        xdo("mousemove", "--window", window, "58", "179", "click", "1")
        xdo("key", "Tab", "Tab", "shift+Tab")
        xdo("type", "Edited ")
        xdo("key", "Tab", "Tab", "Tab")
        xdo("type", "First")
        xdo("key", "Tab")
        xdo("type", "Second|value")
        xdo("key", "Tab")
        filled = (original.replace(b"| B |", b"| Edited B |")
                          .replace(b"| x |", b"| x | First | Second\\|value |"))
        appended = filled.replace(b"Second\\|value |", b"Second\\|value |\r\n|   |   |   |")
        save(appended)
        xdo("type", "Appended")
        save(appended.replace(b"|   |   |   |", b"|   Appended|   |   |"))
        if export:
            subprocess.run(["import", "-window", window, str(export)], check=True, timeout=5)
        xdo("key", "ctrl+z")
        save(appended)
        xdo("key", "ctrl+z")
        save(filled)


def check_link_editing(exe, fixture, export):
    env = dict(os.environ, XDG_CONFIG_HOME=str(fixture.parent / "link-edit-config"))
    original = (b'[**Ref** and *more*][id] [Other][id].\r\n\r\n'
                b'[id]: old.md "Shared tip"\r\n')
    fixture.write_bytes(original)
    with editor_window(exe, fixture, env) as window:
        xdo("key", "ctrl+3")
        until(lambda: mode_is(window, "rich"), "WYSIWYG did not activate")

        def open_link(expected):
            find_text("more")
            xdo("key", "ctrl+k")
            with clipboard_text("Waiting for the link address"):
                def copied():
                    xdo("key", "ctrl+a", "ctrl+c")
                    return subprocess.run(["xclip", "-selection", "clipboard", "-o"],
                                          capture_output=True, timeout=2).stdout == expected
                until(copied, "the link address was not prefilled")

        def save(expected):
            def written():
                xdo("key", "ctrl+s")
                return fixture.read_bytes() == expected

            until(written, "the link edit did not save")
            time.sleep(.2)

        open_link(b"old.md")
        xdo("type", "new file.md")
        xdo("key", "Return")
        edited = original.replace(b'[**Ref** and *more*][id]', b'[**Ref** and *more*](new%20file.md "Shared tip")')
        save(edited)
        open_link(b"new%20file.md")
        if export:
            subprocess.run(["import", "-window", window, str(export)], check=True, timeout=5)
        xdo("key", "Tab", "Return")
        save(original.replace(b"[**Ref** and *more*][id]", b"**Ref** and *more*"))
        xdo("key", "ctrl+z")
        save(edited)
        xdo("key", "ctrl+z")
        save(original)


def check_image(exe, folder, export):
    project = folder / "reports"
    project.mkdir()
    document = project / "Plan.md"
    initial = "# Plan\r\n\r\nInsert after this paragraph.\r\n\r\nKeep the next paragraph.\r\n"
    document.write_bytes(initial.encode("utf-8"))
    picked = folder / "Figure #1.svg"
    picked.write_text(
        '<svg xmlns="http://www.w3.org/2000/svg" width="480" height="180">'
        '<rect width="480" height="180" rx="14" fill="#dbe8f8"/>'
        '<rect x="26" y="35" width="186" height="110" rx="10" fill="#b5358a"/>'
        '<rect x="268" y="35" width="186" height="110" rx="10" fill="#205cb0"/>'
        '<path d="M220 90H260" stroke="#263347" stroke-width="4"/></svg>',
        encoding="utf-8",
    )
    settings = folder / "image-config" / "pane-md" / "settings.json"
    settings.parent.mkdir(parents=True)
    settings.write_text('{"english":true,"theme":"light"}', encoding="utf-8")
    env = dict(os.environ, XDG_CONFIG_HOME=str(settings.parent.parent), GSETTINGS_BACKEND="memory")

    def picker():
        response = xdo("search", "--onlyvisible", "--name", "Insert image")
        return response.stdout.splitlines()[0] if response.stdout else None

    with editor_window(exe, document, env) as window:
        xdo("key", "ctrl+3")
        def rich_mode_visible():
            if mode_is(window, "rich"):
                return True
            xdo("key", "ctrl+3")
            return False

        until(rich_mode_visible, "WYSIWYG did not become active")
        xdo("mousemove", "--window", window, "170", "230", "click", "1")
        xdo("key", "ctrl+shift+k")
        chooser = until(picker, "the native image chooser did not open")
        assert xdo("windowfocus", "--sync", chooser).returncode == 0
        xdo("key", "ctrl+l")
        time.sleep(.25)
        with clipboard_text(str(picked)):
            xdo("key", "ctrl+v")
            time.sleep(.2)
            xdo("key", "Return")
            until(lambda: (project / picked.name).is_file(), "the external image was not copied")
        assert xdo("windowfocus", "--sync", window).returncode == 0

        screenshot = folder / "image.png"
        def visible():
            subprocess.run(["import", "-window", window, str(screenshot)], check=True, timeout=5)
            colors = subprocess.check_output([
                "convert", str(screenshot), "-depth", "8", "-format", "%c", "histogram:info:",
            ], timeout=5)
            return b"#B5358A" in colors

        until(visible, "the SVG was not shown in WYSIWYG")
        if export:
            export.write_bytes(screenshot.read_bytes())
        expected = initial.replace(
            "Insert after this paragraph.",
            "Insert after this paragraph.\r\n\r\n![Figure #1](Figure%20%231.svg)",
        )
        xdo("key", "ctrl+s")
        until(lambda: document.read_bytes() == expected.encode("utf-8"),
              "the inserted image was not saved as a relative Markdown link")
        xdo("key", "ctrl+z")
        time.sleep(.35)
        xdo("key", "ctrl+s")
        until(lambda: document.read_bytes() == initial.encode("utf-8"),
              "undo did not restore the original document")
        assert (project / picked.name).read_bytes() == picked.read_bytes()


def check_replace(exe, folder, export):
    fixture = folder / "Replace QA.md"
    original = "\ufeff# Plan\r\n\r\n😀 word WORD sword.\r\n\r\n<!-- pane-md:review word invalid -->\r\n".encode()
    fixture.write_bytes(original)
    settings = folder / "replace-config" / "pane-md" / "settings.json"
    settings.parent.mkdir(parents=True)
    settings.write_text('{"english":true,"theme":"light"}', encoding="utf-8")
    env = dict(os.environ, XDG_CONFIG_HOME=str(settings.parent.parent))

    with editor_window(exe, fixture, env) as window:
        def click(x, y):
            assert xdo("mousemove", "--window", window, str(x), str(y), "click", "1").returncode == 0

        def saved(expected, explanation):
            def save():
                xdo("key", "ctrl+s")
                return fixture.read_bytes() == expected

            until(save, explanation)

        until(lambda: mode_is(window, "source"), "Markdown editor did not become active")
        xdo("key", "ctrl+h")
        until(lambda: b"#FBFAF8" in pixel_at(window, 40, 170), "replacement field did not open")
        click(650, 140)  # Whole words
        click(100, 140)
        xdo("type", "word")
        xdo("key", "F3")
        click(100, 170)
        xdo("type", "note")
        if export:
            time.sleep(.15)
            subprocess.run(["import", "-window", window, str(export)], check=True, timeout=5)

        xdo("key", "Return")
        saved(original.replace(b"WORD", b"note"), "the selected uppercase hit was not replaced")
        xdo("key", "Escape", "ctrl+z")
        saved(original, "undo did not restore the first replacement")

        xdo("key", "ctrl+h")
        click(100, 140)
        xdo("key", "ctrl+a")
        xdo("type", "word")
        click(100, 170)
        xdo("type", "note")
        xdo("key", "ctrl+Return")
        saved(original.replace(b"word WORD sword", b"note note sword"),
              "replace all changed a partial word or a protected review record")
        xdo("key", "Escape", "ctrl+z")
        saved(original, "replace all was not undone in one step")


def check_folder(exe, folder, export):
    project = folder / "Notes"
    archive = project / "docs" / "archive"
    archive.mkdir(parents=True)
    nested = archive / "Baseline.md"
    nested.write_text("# Weekly project baseline and delivery notes\n\nThe nested project note.\n", encoding="utf-8")
    (archive.parent / "Specification.md").write_text("# Specification\n", encoding="utf-8")
    first = project / "Plan.md"
    second = project / "Tasks.MD"
    original = "# Plan\n\nThe delivery notes.\n"
    first.write_text(original, encoding="utf-8")
    settings = folder / "folder-config" / "pane-md" / "settings.json"
    settings.parent.mkdir(parents=True)
    settings.write_text('{"english":true,"theme":"light"}', encoding="utf-8")
    env = dict(os.environ, XDG_CONFIG_HOME=str(settings.parent.parent))

    with editor_window(exe, project, env, wait=False) as window:
        def click(x, y):
            assert xdo("mousemove", "--window", window, str(x), str(y), "click", "1").returncode == 0

        def title(name):
            return xdo("getwindowname", window).stdout.strip() == name + " — pane-md"

        def row_visible(y):
            colors = subprocess.check_output([
                "import", "-window", window, "-crop", f"130x22+8+{y}", "-format", "%k", "info:",
            ], timeout=5)
            return int(colors) > 1

        until(lambda: row_visible(222), "folder entries did not appear")
        click(54, 233)
        until(lambda: title(first.name), "the first file did not open from the folder")

        def source_active():
            if mode_is(window, "source"):
                return True
            xdo("key", "ctrl+4")
            return False

        until(source_active, "Markdown editor did not become active")
        xdo("key", "ctrl+End")
        xdo("type", "Keep this edit.")
        second.write_text("# Tasks\n\n- [ ] Review plan\n", encoding="utf-8")
        xdo("key", "F5")
        until(lambda: row_visible(253), "F5 did not list the new Markdown file")
        assert title(first.name)
        assert first.read_text(encoding="utf-8") == original
        click(54, 264)
        until(lambda: title(second.name), "the new file did not open from the folder")
        click(54, 233)
        until(lambda: title(first.name), "the already open file did not regain its tab")
        time.sleep(.2)

        def saved():
            xdo("key", "ctrl+s")
            return first.read_text(encoding="utf-8") == original + "Keep this edit."

        until(saved, "folder refresh or tab switch lost an unsaved edit")
        xdo("key", "ctrl+Tab")
        until(lambda: title(second.name), "opening the existing file created a duplicate tab")
        second.unlink()
        time.sleep(.2)
        xdo("key", "F5")
        until(lambda: not row_visible(253), "F5 did not remove the deleted file from the folder")
        assert title(second.name)

        click(54, 204)
        until(lambda: row_visible(283), "expanding docs hid its sibling file or did not show its children")
        xdo("key", "--delay", "350", "Right", "Right", "Right")
        until(lambda: row_visible(313), "arrow keys did not expand the nested folder")
        latest = archive / "A latest.md"
        latest.write_text("# Latest\n", encoding="utf-8")
        (project / "ideas").mkdir()
        xdo("key", "F5")
        until(lambda: row_visible(373), "F5 did not preserve both expanded folders and add the new rows")
        xdo("key", "Return")
        until(lambda: title(nested.name), "F5 lost the focused file after new siblings changed the row indices")

        click(76, 264)
        until(lambda: title(latest.name), "the new nested file did not open from its own tree row")
        if export:
            subprocess.run(["import", "-window", window, str(export)], check=True, timeout=5)
        click(54, 204)
        until(lambda: not row_visible(313), "collapsing docs left its descendants visible")
        click(54, 204)
        until(lambda: row_visible(373), "reopening docs did not restore its expanded archive")
        click(76, 294)
        until(lambda: title(nested.name), "the restored nested folder did not activate the existing tab")
        click(189, 98)
        until(lambda: b"#FBFAF8" in pixel_at(window, 100, 500), "the file pane did not collapse")
        click(23, 98)
        until(lambda: row_visible(373), "the edge button opened the outline instead of the file tree")
        xdo("key", "ctrl+1")
        xdo("windowsize", window, "620", "670")
        until(lambda: b"#F1F0EE" in pixel_at(window, 100, 500), "the file tree did not fit in a compact window")
        click(580, 98)
        xdo("key", "--delay", "80", "End", "Return")
        until(lambda: b"#F1F0EE" in pixel_at(window, 607, 500),
              "the compact window clipped the document mode selector")


def check_document_navigation(exe, folder):
    settings = folder / "document-navigation-config" / "pane-md" / "settings.json"
    settings.parent.mkdir(parents=True)
    settings.write_text('{"english":true,"theme":"light"}', encoding="utf-8")
    env = dict(os.environ, XDG_CONFIG_HOME=str(settings.parent.parent))
    fixture = folder / "Navigation commands.md"
    original = "# Overview\n\nTarget in a paragraph.\n\n## Target One\n\n**bold target**\n\n## Target Two\n"
    fixture.write_text(original, encoding="utf-8")

    with editor_window(exe, fixture, env, wait=False) as window:
        def click(x, y):
            assert xdo("mousemove", "--window", window, str(x), str(y), "click", "1").returncode == 0

        def view_command(y):
            click(153, 16)
            time.sleep(.15)
            click(179, y)
            time.sleep(.2)

        def copy_selection(expected, *keys):
            with clipboard_text("Waiting for navigation selection"):
                xdo("key", *keys, "ctrl+c")
                until(lambda: subprocess.run(["xclip", "-selection", "clipboard", "-o"],
                                            capture_output=True, timeout=2).stdout == expected.encode(),
                      f"navigation selection did not become {expected!r}")

        until(lambda: mode_is(window, "rich"), "the document did not open in WYSIWYG")
        xdo("key", "ctrl+3")
        time.sleep(.3)
        find_text("bold target")
        view_command(76)  # Show in Markdown source.
        until(lambda: mode_is(window, "source"), "View > Show in Markdown source did not switch views")
        copy_selection("bold target")

        xdo("key", "ctrl+End", "Return")
        xdo("type", "## Target Three")
        view_command(50)  # Find heading.
        xdo("type", "target")
        xdo("key", "--delay", "320", "Return", "F3", "F3", "Escape")
        copy_selection("## Target Three", "Home", "shift+End")

        view_command(50)
        xdo("type", "target")
        xdo("key", "--delay", "320", "shift+F3", "Escape")
        copy_selection("## Target Two", "Home", "shift+End")
        assert fixture.read_text(encoding="utf-8") == original

        click(24, 98)  # Open the outline and focus its toggle.
        until(lambda: b"#F1F0EE" in pixel_at(window, 100, 500), "outline did not open")
        xdo("key", "shift+F6", "ctrl+Home", "F6", "Return")
        copy_selection("## Target Two", "Home", "shift+End")
        xdo("key", "F6", "F6", "Return")
        until(lambda: b"#FBFAF8" in pixel_at(window, 1135, 500), "F6 did not reach the comments pane")

        xdo("key", "ctrl+1")
        until(lambda: mode_is(window, "rich"), "reading mode did not open in WYSIWYG")
        view_command(50)
        xdo("type", "target two")
        xdo("key", "--delay", "320", "Return", "Escape")
        view_command(76)
        until(lambda: mode_is(window, "source"), "the WYSIWYG heading did not open in source")
        copy_selection("## Target Two")


def check_lint_navigation(exe, folder):
    settings = folder / "lint-config" / "pane-md" / "settings.json"
    settings.parent.mkdir(parents=True)
    settings.write_text('{"english":true,"theme":"light"}', encoding="utf-8")
    env = dict(os.environ, XDG_CONFIG_HOME=str(settings.parent.parent))
    fixture = folder / "Lint workspace.md"
    (folder / "Target note.md").write_text("# Present\n", encoding="utf-8")
    heading = "[Missing heading](Target%20note.md#absent)"
    image = "![Missing image](not-here.png)"
    original = f"# Local lint\n\n[Valid](Target%20note.md#present)\n\n{heading}\n\n{image}\n"
    fixture.write_text(original, encoding="utf-8")

    with editor_window(exe, fixture, env) as window:
        def assert_line(expected):
            with clipboard_text("Waiting for lint result"):
                xdo("key", "Home", "shift+End", "ctrl+c")
                until(lambda: subprocess.run(["xclip", "-selection", "clipboard", "-o"],
                                            capture_output=True, timeout=2).stdout == expected.encode(),
                      f"lint did not jump to {expected!r}")

        until(lambda: mode_is(window, "source"), "the lint fixture did not open in Markdown")
        for shortcut, expected in [("F8", heading), ("F8", image), ("shift+F8", heading)]:
            xdo("key", shortcut)
            time.sleep(.35)
            assert_line(expected)

        xdo("key", "ctrl+1")
        until(lambda: mode_is(window, "rich"), "WYSIWYG reading did not open")
        xdo("key", "--delay", "320", "F8", "F8")
        assert xdo("mousemove", "--window", window, "153", "16", "click", "1").returncode == 0
        time.sleep(.15)
        assert xdo("mousemove", "--window", window, "179", "76", "click", "1").returncode == 0
        until(lambda: mode_is(window, "source"), "the WYSIWYG lint result did not open in Markdown")
        xdo("key", "ctrl+4")
        time.sleep(.3)
        assert_line(image)
    assert fixture.read_text(encoding="utf-8") == original


def check_link_navigation(exe, folder, export):
    project = folder / "Link project"
    project.mkdir()
    origin = project / "Plan.md"
    target = project / "Target #1.md"
    heading_link = "[Go to section](#далёкий-раздел)"
    other_link = "[Open target](Target%20%231.md#нужный-раздел)"
    origin.write_text(
        "# Plan\n\n" + heading_link + "\n\n" + other_link + "\n\nReturn here[^details].\n\n"
        + "\n\n".join(f"Paragraph {i} keeps the destination offscreen." for i in range(1, 35))
        + "\n\n## Далёкий раздел\n\nLast section.\n\n[^details]: Notes for the project.\n",
        encoding="utf-8",
    )
    target.write_text(
        "# Target\n\n" + "\n\n".join(f"Linked document paragraph {i}." for i in range(1, 26))
        + "\n\n## Нужный раздел\n\nA different Markdown file.\n",
        encoding="utf-8",
    )
    settings = folder / "link-config" / "pane-md" / "settings.json"
    settings.parent.mkdir(parents=True)
    settings.write_text('{"english":true,"theme":"light"}', encoding="utf-8")
    env = dict(os.environ, XDG_CONFIG_HOME=str(settings.parent.parent))

    with editor_window(exe, project, env, wait=False) as window:
        def title(name):
            return xdo("getwindowname", window).stdout.strip() == name + " — pane-md"

        def activate_mode(shortcut, family):
            def active():
                if mode_is(window, family):
                    return True
                xdo("key", shortcut)
                return False

            until(active, f"mode for {shortcut} did not become active")

        def copy_selection(expected, keys=()):
            def copied():
                xdo("key", *keys, "ctrl+c")
                return subprocess.run(["xclip", "-selection", "clipboard", "-o"],
                                      capture_output=True, timeout=2).stdout == expected.encode()

            with clipboard_text("Waiting for editor selection"):
                until(copied, f"editor selection was not {expected!r}")

        def body():
            return subprocess.check_output([
                "import", "-window", window, "-crop", "440x235+273+330", "-depth", "8", "rgb:-",
            ], timeout=5)

        until(lambda: b"#D5D4D1" in pixel_at(window, 215, 420), "file pane did not appear")
        xdo("mousemove", "--window", window, "50", "204", "click", "1")
        until(lambda: title(origin.name), "origin file did not open")
        activate_mode("ctrl+4", "source")
        copy_selection("# Plan", ("Home", "shift+End"))
        xdo("mousemove", "--window", window, "315", "192", "keydown", "ctrl", "click", "1", "keyup", "ctrl")
        copy_selection("## Далёкий раздел", ("Home", "shift+End"))
        xdo("key", "alt+Left")
        copy_selection(heading_link, ("Home", "shift+End"))
        xdo("key", "alt+Right")
        copy_selection("## Далёкий раздел", ("Home", "shift+End"))
        xdo("key", "alt+Left")
        copy_selection(heading_link, ("Home", "shift+End"))

        find_text("Open target")
        xdo("key", "ctrl+Return")
        until(lambda: title(target.name), "the encoded filename did not open in its own tab")
        time.sleep(.25)
        xdo("key", "ctrl+4")
        copy_selection("## Нужный раздел", ("Home", "shift+End"))
        activate_mode("ctrl+1", "rich")
        find_text("A different Markdown file.")
        xdo("key", "alt+Left")
        until(lambda: title(origin.name), "Back did not return to the Markdown tab")
        copy_selection("Open target")
        xdo("key", "alt+Right")
        until(lambda: title(target.name), "Forward did not open the linked tab")
        copy_selection("A different Markdown file.")
        xdo("key", "alt+Left")
        until(lambda: title(origin.name), "a second Back did not restore the Markdown tab")
        copy_selection("Open target")

        activate_mode("ctrl+1", "rich")
        find_text("[^details]")
        first_view = body()
        xdo("key", "ctrl+Return")
        until(lambda: body() != first_view, "Ctrl+Enter did not scroll to the footnote")

        with clipboard_text("↩"):
            xdo("key", "ctrl+f")
            time.sleep(.18)
            xdo("key", "ctrl+v")
            time.sleep(.2)
            xdo("key", "Escape")
            time.sleep(.12)
        copy_selection("↩")
        xdo("key", "ctrl+Return")
        until(lambda: body() == first_view, "the footnote return link did not reach its first reference")

        xdo("key", "alt+Left")
        until(lambda: body() != first_view, "Back did not revisit the footnote")
        xdo("key", "alt+Left")
        until(lambda: body() == first_view, "Back did not restore the WYSIWYG viewport")
        xdo("key", "alt+Right")
        until(lambda: body() != first_view, "Forward did not scroll to the WYSIWYG footnote again")
        xdo("key", "alt+Left")
        until(lambda: body() == first_view, "Back did not restore the viewport after Forward")

        find_text("Open target")
        xdo("key", "ctrl+Return")
        until(lambda: title(target.name), "Ctrl+Enter did not follow the WYSIWYG link")
        xdo("key", "alt+Right")
        time.sleep(.25)
        assert title(target.name), "a new link left a stale Forward destination"
        if export:
            time.sleep(.2)
            subprocess.run(["import", "-window", window, str(export)], check=True, timeout=5)


def check_new_note(exe, folder, export):
    project = folder / "New note project"
    project.mkdir()
    expected = project / "First note.md"
    settings = folder / "draft-config" / "pane-md" / "settings.json"
    settings.parent.mkdir(parents=True)
    settings.write_text('{"english":true,"theme":"light"}', encoding="utf-8")
    env = dict(os.environ, XDG_CONFIG_HOME=str(settings.parent.parent), GSETTINGS_BACKEND="memory")

    with editor_window(exe, project, env, wait=False) as window:
        def title(name):
            return xdo("getwindowname", window).stdout.strip() == name + " — pane-md"

        def chooser():
            response = xdo("search", "--onlyvisible", "--name", "^Save as$")
            return response.stdout.splitlines()[0] if response.stdout else None

        def open_save():
            xdo("key", "ctrl+s")
            dialog = until(chooser, "first save did not offer a filename")
            assert xdo("windowfocus", "--sync", dialog).returncode == 0
            return dialog

        until(lambda: b"#D5D4D1" in pixel_at(window, 215, 420), "file pane did not appear")
        xdo("key", "ctrl+n")
        until(lambda: title("Untitled 1"), "Ctrl+N did not open the first draft")
        until(lambda: mode_is(window, "rich"), "a new note did not start in WYSIWYG")
        xdo("type", "First draft")
        xdo("key", "ctrl+n")
        until(lambda: title("Untitled 2"), "Ctrl+N did not open a second draft")
        time.sleep(.2)
        xdo("type", "Second draft")
        xdo("key", "ctrl+shift+Tab")  # gitleaks:allow - xdotool key chord, not a credential
        until(lambda: title("Untitled 1"), "switching tabs did not recover the first draft")
        assert not list(project.iterdir()), "creating or editing a draft already wrote a file"

        time.sleep(.25)
        open_save()
        xdo("key", "Escape")
        until(lambda: chooser() is None, "the save dialog did not cancel")
        assert xdo("windowfocus", "--sync", window).returncode == 0
        assert title("Untitled 1")
        assert not list(project.iterdir()), "cancelling a draft's first save wrote a file"
        time.sleep(.25)
        xdo("type", " kept")
        if export:
            time.sleep(.15)
            subprocess.run(["import", "-window", window, str(export)], check=True, timeout=5)

        open_save()
        xdo("key", "ctrl+a")
        xdo("type", "First note.md")
        xdo("key", "Return")
        until(expected.is_file, "the new note did not save to the selected directory")
        until(lambda: title(expected.name), "first save did not rename the original tab")
        assert expected.read_text(encoding="utf-8") == "First draft kept"
        assert xdo("windowfocus", "--sync", window).returncode == 0
        time.sleep(.2)
        xdo("key", "ctrl+Tab")
        until(lambda: title("Untitled 2"), "first save replaced the other draft's tab")
        time.sleep(.2)
        open_save()
        xdo("key", "ctrl+a")
        xdo("type", "Second note.md")
        xdo("key", "Return")
        second = project / "Second note.md"
        until(second.is_file, "the second draft did not save")
        assert second.read_text(encoding="utf-8") == "Second draft"


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("binary", type=Path)
    parser.add_argument("--export-review", type=Path)
    parser.add_argument("--export-mermaid", type=Path)
    parser.add_argument("--export-appearance", type=Path)
    parser.add_argument("--export-formatting", type=Path)
    parser.add_argument("--export-paste", type=Path)
    parser.add_argument("--export-visual-replacement", type=Path)
    parser.add_argument("--export-structured-replacement", type=Path)
    parser.add_argument("--export-sparse-table", type=Path)
    parser.add_argument("--export-table-keyboard", type=Path)
    parser.add_argument("--export-link-editing", type=Path)
    parser.add_argument("--export-image", type=Path)
    parser.add_argument("--export-replace", type=Path)
    parser.add_argument("--export-folder", type=Path)
    parser.add_argument("--export-navigation", type=Path)
    parser.add_argument("--export-new-note", type=Path)
    parser.add_argument("--export-workspace", type=Path)
    args = parser.parse_args()
    exe = str(args.binary.resolve(strict=True))
    with tempfile.TemporaryDirectory() as directory:
        fixture = Path(directory) / "Portable QA.md"
        original = "\ufeff# План\r\n\r\nKeep **formatting** and emoji 😀.\r\n".encode()
        fixture.write_bytes(original)
        check_window(exe, fixture, original)
        check_large_source(exe, Path(directory) / "Large source.md")
        review = Path(directory) / "Portable review QA.md"
        original = "\ufeff# Review\r\n\r\nFirst annotated text.\r\n\r\nSecond paragraph.\r\n".encode()
        review.write_bytes(original)
        check_reviews(exe, review, original)
        folder = Path(directory)
        check_mcp_open(exe, folder)
        check_workspace(exe, folder, args.export_workspace)
        check_mermaid(exe, folder / "Mermaid.md", folder / "view.png", args.export_mermaid)
        check_appearance(exe, folder / "Appearance.md", args.export_appearance)
        check_formatting(exe, folder / "Formatting.md", args.export_formatting)
        check_paste(exe, folder / "Clipboard.md", args.export_paste)
        check_visual_replacement(exe, folder / "Visual replacement.md", args.export_visual_replacement)
        check_structured_replacement(exe, folder / "Structured edit.md", args.export_structured_replacement)
        check_sparse_tables(exe, folder / "Sparse table.md", args.export_sparse_table)
        check_table_keyboard(exe, folder / "Table keyboard.md", args.export_table_keyboard)
        check_link_editing(exe, folder / "Edit link.md", args.export_link_editing)
        check_image(exe, folder, args.export_image)
        check_replace(exe, folder, args.export_replace)
        check_folder(exe, folder, args.export_folder)
        check_document_navigation(exe, folder)
        check_lint_navigation(exe, folder)
        check_link_navigation(exe, folder, args.export_navigation)
        check_new_note(exe, folder, args.export_new_note)
        if args.export_review:
            args.export_review.write_bytes(review.read_bytes())
    print("Linux WebKit MCP launch and shutdown, workspace layouts, new notes, editing across paragraphs, list items and table cells, table keyboard navigation, sparse tables, tabs and folder refresh, heading search, source jumps, pane focus, link navigation and editing, lint navigation, find and replace, comments, Mermaid, appearance, formatting, paste, image import, BOM, and CRLF verified.")


if __name__ == "__main__":
    main()
