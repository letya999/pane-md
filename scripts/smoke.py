
"""Check Windows process lifetime, MCP framing and startup measurements."""
import argparse
import ctypes as c
from ctypes import wintypes as w
import json
from pathlib import Path
import statistics
import subprocess
import tempfile
import time

PARSER = argparse.ArgumentParser(description=__doc__)
PARSER.add_argument("exe", type=Path)
PARSER.add_argument("fixture", type=Path)
PARSER.add_argument("--typing", action="store_true", help="measure WYSIWYG input on a temporary copy")
args = PARSER.parse_args()
exe = str(args.exe.resolve(strict=True))
fixture = args.fixture.resolve(strict=True)

USER = c.WinDLL("user32", use_last_error=True)
KERNEL = c.WinDLL("kernel32", use_last_error=True)
PSAPI = c.WinDLL("psapi", use_last_error=True)
ENUM = c.WINFUNCTYPE(w.BOOL, w.HWND, w.LPARAM)
USER.EnumWindows.argtypes = [ENUM, w.LPARAM]
USER.GetWindowTextW.argtypes = [w.HWND, w.LPWSTR, c.c_int]
USER.GetWindowThreadProcessId.argtypes = [w.HWND, c.POINTER(w.DWORD)]
USER.IsWindowVisible.argtypes = [w.HWND]
USER.PostMessageW.argtypes = [w.HWND, w.UINT, w.WPARAM, w.LPARAM]
USER.SendMessageW.argtypes = [w.HWND, w.UINT, w.WPARAM, w.LPARAM]
USER.SendMessageW.restype = w.LPARAM
USER.FindWindowExW.argtypes = [w.HWND, w.HWND, w.LPCWSTR, w.LPCWSTR]
USER.FindWindowExW.restype = w.HWND
USER.GetWindowLongW.argtypes = [w.HWND, c.c_int]
USER.UpdateWindow.argtypes = [w.HWND]
USER.ShowWindow.argtypes = [w.HWND, c.c_int]
USER.SetWindowPos.argtypes = [w.HWND, w.HWND, c.c_int, c.c_int, c.c_int, c.c_int, w.UINT]
KERNEL.OpenProcess.argtypes = [w.DWORD, w.BOOL, w.DWORD]
KERNEL.OpenProcess.restype = w.HANDLE
KERNEL.WaitForSingleObject.argtypes = [w.HANDLE, w.DWORD]
KERNEL.CloseHandle.argtypes = [w.HANDLE]
USER.WaitForInputIdle.argtypes = [w.HANDLE, w.DWORD]

class Memory(c.Structure):
    _fields_ = [("size", w.DWORD), ("faults", w.DWORD)] + [
        (name, c.c_size_t) for name in (
            "peak_working", "working", "peak_paged", "paged",
            "peak_nonpaged", "nonpaged", "pagefile", "peak_pagefile",
        )
    ]

PSAPI.GetProcessMemoryInfo.argtypes = [w.HANDLE, c.POINTER(Memory), w.DWORD]

def windows(title, match_pid=None):
    found = []

    @ENUM
    def visit(hwnd, _):
        if USER.IsWindowVisible(hwnd):
            name = c.create_unicode_buffer(512)
            USER.GetWindowTextW(hwnd, name, len(name))
            if name.value == title:
                pid = w.DWORD()
                USER.GetWindowThreadProcessId(hwnd, c.byref(pid))
                if match_pid is None or match_pid == pid.value:
                    found.append((hwnd, pid.value))
        return True

    USER.EnumWindows(visit, 0)
    return found

def wait_window(title, excluded=(), pid=None):
    end = time.monotonic() + 12
    while time.monotonic() < end:
        found = [pair for pair in windows(title, pid) if pair[1] not in excluded]
        if found:
            return found[0]
        time.sleep(.008)
    raise AssertionError(f"timed out finding {title!r}")

def close_window(hwnd, pid):
    with_handle = KERNEL.OpenProcess(0x100000, False, pid)
    assert with_handle
    try:
        assert USER.PostMessageW(hwnd, 0x0010, 0, 0)
        assert KERNEL.WaitForSingleObject(with_handle, 10_000) == 0, f"GUI process {pid} survived window closure"
    finally:
        KERNEL.CloseHandle(with_handle)

def measure(path):
    title = "notepad-md" if path is None else f"{path.name} — notepad-md"
    existing = {pid for _, pid in windows(title)}
    start = time.perf_counter()
    subprocess.run([exe, *([str(path)] if path else [])], check=True, timeout=5)
    hwnd, pid = wait_window(title, existing)
    handle = KERNEL.OpenProcess(0x101410, False, pid)
    assert handle
    try:
        assert USER.WaitForInputIdle(handle, 3000) == 0
        milliseconds = (time.perf_counter() - start) * 1000
        time.sleep(1)
        memory = Memory()
        memory.size = c.sizeof(memory)
        assert PSAPI.GetProcessMemoryInfo(handle, c.byref(memory), memory.size)
        return (round(milliseconds, 1), round(memory.working / 1024**2, 2))
    finally:
        KERNEL.CloseHandle(handle)
        close_window(hwnd, pid)

def review_fixture(path):
    record = {
        "v": 1, "id": "smoke", "quote": "P85",
        "anchor": {"heading": "Lead time", "block": "table", "row": 1, "cell": 0, "prefix": "Ж😀 ", "suffix": " by team"},
        "text": "Проверка Ж😀 -->\nВторая строка.", "status": "open",
    }
    line = json.dumps(record, ensure_ascii=False, separators=(",", ":")).replace("--", "\\u002D\\u002D")
    tag = f"<!-- notepad-md:review {line} -->"
    path.write_bytes(("# Lead time\n\n| X | Y |\n| --- | --- |\n| Ж😀 **P85** by team | P85 |\n\n"
                      f"{tag}\n\n```md\n{tag}\n```\n\n<!-- notepad-md:review {{broken}} -->\n"
                      "<!-- external html comment -->\n").encode("utf-8"))
    return record

def set_control_text(hwnd, text):
    buffer = c.create_unicode_buffer(text)
    assert USER.SendMessageW(hwnd, 0x000C, 0, c.addressof(buffer))

def command(hwnd, identifier):
    USER.SendMessageW(hwnd, 0x0111, identifier, 0)

def create_gui_review(folder):
    path = folder / "Ревью из GUI.md"
    content = "# Lead time\n\n| Metric | Value | Note |\n| --- | --- | --- |\n| P85 | 7 | Ж😀 |\n"
    path.write_bytes(content.encode("utf-8"))
    subprocess.run([exe, str(path)], check=True, timeout=5)
    hwnd, pid = wait_window(f"{path.name} — notepad-md")
    try:
        editor = USER.FindWindowExW(hwnd, None, "RICHEDIT50W", None)
        assert editor
        prefix = "Lead time\r\ufff9\rMetric\x07Value\x07Note\x07\ufffb\r\ufff9\r"
        offset = len(prefix.encode("utf-16-le")) // 2
        USER.SendMessageW(editor, 0x00B1, offset, offset + 3)
        command(hwnd, 117)  # Add review from the native selection.
        child = None
        while True:
            child = USER.FindWindowExW(hwnd, child, "EDIT", None)
            if not child:
                raise AssertionError("Review composer did not open")
            style = USER.GetWindowLongW(child, -16)
            if USER.IsWindowVisible(child) and style & 0x0004 and not style & 0x0800:
                break
        note = "Проверка из GUI Ж😀 -->"
        set_control_text(child, note)
        command(hwnd, 123)
        command(hwnd, 103)
        saved = path.read_bytes().decode("utf-8")
        assert saved.count("<!-- notepad-md:review ") == 1
        start = saved.index("\n\n<!-- notepad-md:review ")
        end = saved.index("-->", start) + 3
        assert saved[:start] + saved[end:] == content
        return path, note
    finally:
        close_window(hwnd, pid)

def check_mcp(folder):
    path = folder / "Комментарий теста.md"
    expected = review_fixture(path)
    content = path.read_bytes()
    title = f"{path.name} — notepad-md"
    gui_path, gui_note = create_gui_review(folder)
    with subprocess.Popen([exe, "--mcp"], stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=subprocess.PIPE,
                          text=True, encoding="utf-8", cwd=folder) as server:
        number = 0

        def request(method, params=None, notice=False):
            nonlocal number
            message = {"jsonrpc": "2.0", "method": method}
            if params is not None:
                message["params"] = params
            if not notice:
                number += 1
                message["id"] = number
            server.stdin.write(json.dumps(message) + "\n")
            server.stdin.flush()
            if not notice:
                response = json.loads(server.stdout.readline())
                assert response["id"] == number, response
                return response

        assert "result" in request("initialize", {"protocolVersion": "2025-11-25", "capabilities": {}, "clientInfo": {"name": "smoke", "version": "1"}})
        request("notifications/initialized", notice=True)
        assert {tool["name"] for tool in request("tools/list")["result"]["tools"]} == {"open_document", "list_comments"}

        def call(name, path):
            return request("tools/call", {"name": name, "arguments": {"path": str(path)}})

        assert "error" in call("list_comments", "missing.md")
        assert call("list_comments", fixture)["result"]["structuredContent"]["comments"] == []
        result = call("list_comments", path.name)["result"]["structuredContent"]
        [record] = result["comments"]
        assert record["id"] == expected["id"] and record["quote"] == expected["quote"] and record["text"] == expected["text"]
        assert record["state"] == "attached" and record["row"] == 1 and record["line"] == 5
        assert len(result["diagnostics"]) == 1
        gui_result = call("list_comments", gui_path)["result"]["structuredContent"]
        [from_gui] = gui_result["comments"]
        assert from_gui["quote"] == "P85" and from_gui["text"] == gui_note
        assert from_gui["state"] == "attached" and from_gui["row"] == 1 and from_gui["cell"] == 0
        opened = call("open_document", path.name)["result"]["structuredContent"]
        assert Path(opened["path"]) == path and opened["status"] == "launched"
        hwnd, pid = wait_window(title)
        assert pid != server.pid and server.poll() is None
        close_window(hwnd, pid)
        assert path.read_bytes() == content
        server.stdin.close()
        assert server.wait(timeout=5) == 0
        assert "notepad-md MCP" in server.stderr.read()

def check_wait(folder):
    for name in ("Промпт с пробелами.txt", "Промпт без расширения"):
        path = folder / name
        data = "Временный промпт.\r\n".encode("utf-8")
        path.write_bytes(data)
        process = subprocess.Popen([exe, "--source", "--wait", name], cwd=folder)
        hwnd, pid = wait_window(f"{name} — notepad-md", pid=process.pid)
        assert process.poll() is None
        editor = USER.FindWindowExW(hwnd, None, "RICHEDIT50W", None)
        assert editor
        replacement = "Сохранённый промпт Ж😀.\r\n"
        set_control_text(editor, replacement)
        command(hwnd, 103)
        assert process.poll() is None
        close_window(hwnd, pid)
        assert process.wait(timeout=5) == 0
        assert path.read_bytes() == replacement.encode("utf-8")

def measure_typing(directory):
    marker = b"Typing probex"
    path = directory / "Typing benchmark.md"
    original = marker + b"\n\n" + fixture.read_bytes()
    path.write_bytes(original)
    subprocess.run([exe, str(path)], check=True, timeout=5)
    hwnd, pid = wait_window(f"{path.name} — notepad-md")
    try:
        # Keep the benchmark from receiving keyboard input from the interactive desktop.
        USER.ShowWindow(hwnd, 0)
        assert USER.SetWindowPos(hwnd, None, -20_000, -20_000, 0, 0, 0x0015)
        USER.ShowWindow(hwnd, 4)
        editor = USER.FindWindowExW(hwnd, None, "RICHEDIT50W", None)
        assert editor
        command(hwnd, 152)  # WYSIWYG Edit
        USER.SendMessageW(editor, 0x00B1, len(marker), len(marker))
        samples = []
        for trial in range(50):
            started = time.perf_counter_ns()
            USER.SendMessageW(editor, 0x0102, ord("a"), 1)
            command(hwnd, 0)  # Synchronize the edit without writing the file.
            USER.UpdateWindow(editor)
            if trial >= 10:
                samples.append((time.perf_counter_ns() - started) / 1_000_000)
        command(hwnd, 103)
        assert path.read_bytes() == marker + b"a" * 50 + original[len(marker):]
        p95 = sorted(samples)[37]
        print(f"WYSIWYG, {len(original)} bytes, n={len(samples)}: p50={statistics.median(samples):.2f} ms, p95={p95:.2f} ms")
    finally:
        close_window(hwnd, pid)

with tempfile.TemporaryDirectory(prefix="notepad-md-smoke-") as directory:
    if args.typing:
        measure_typing(Path(directory))
    else:
        check_mcp(Path(directory))
        check_wait(Path(directory))

if not args.typing:
    before = fixture.read_bytes()
    for label, target in [("empty", None), ("large", fixture)]:
        observations = [measure(target) for _ in range(3)]
        print(label, "samples (ms, MiB):", observations,
              "medians:", tuple(statistics.median(column) for column in zip(*observations)))
    assert fixture.read_bytes() == before
    print("MCP and CLI lifecycle checks passed; EXE bytes:", Path(exe).stat().st_size)
