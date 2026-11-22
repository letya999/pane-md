"""Run the Markdown source view's browser component checks with Chromium."""

from html import escape, unescape
import json
import os
from pathlib import Path
import re
import shutil
import subprocess
import tempfile

ROOT = Path(__file__).resolve().parents[1] / "src" / "portable"

SCENARIO = r"""
const byId = (id) => document.getElementById(id);
const source = byId("source");
const rows = byId("source-lines").children;
const row = (index) => byId("source-lines").querySelector('[data-line="' + (index + 1) + '"]');
const lines = Array.from({length: 30000}, (_, i) => "Line " + i + " with text");
lines[0] = "# План 😀";
lines[1] = "<!-- review metadata -->";
const initial = lines.join("\n");
const firstOffset = initial.indexOf(lines[5]);
const secondOffset = initial.indexOf(lines[800]);
const review = (id, offset) => ({id, location: {offset, endOffset: offset + 4}});
let focusedReview = "first";
const state = {document: {
  source: initial, showReviews: true, headings: [{offset: 0}],
  sourceRecords: [[lines[0].length + 1, lines[0].length + 1 + lines[1].length]],
  reviews: [review("first", firstOffset), review("second", secondOffset)],
}};
const failures = [];
function check(condition, message) {
  if (!condition) failures.push(message);
}
function update() {
  paintSource();
  // Include style and layout, which a source editor needs to show the new caret.
  return rows[rows.length - 1].offsetTop;
}
try {
  source.value = initial;
  updateSourceLineHeight(23, 100);
  const opening = performance.now();
  update();
  const openMs = performance.now() - opening;
  const initialRows = rows.length;
  check(rows.length <= 256 && !row(18000), "rendering is bounded by the viewport");
  check(!document.querySelector(".source-metrics") && fixedSourceLineHeight === 23,
    "native line height is used for documents without soft wraps");
  check([...row(0).querySelectorAll(".source-heading")].some(span => span.textContent.includes("План 😀")), "UTF-16 heading");
  check(row(1).querySelector(".source-metadata")?.textContent === lines[1], "review metadata");
  check(row(5).querySelector(".source-mark-active")?.textContent === "Line", "active mark");
  check(row(5).querySelector(".source-badge")?.textContent === "1", "active badge");
  focusedReview = "second";
  update();
  check(!row(5).querySelector(".source-badge"), "focus clears the previous badge");
  scrollSourceTo(secondOffset, false);
  check(row(800).querySelector(".source-badge")?.textContent === "2", "the new badge appears when scrolled to");
  check(source.scrollTop > 0 && Math.abs(row(799).getBoundingClientRect().top - source.getBoundingClientRect().top) < 2,
    "navigation scrolls to the right logical row");
  check(rows.length <= 256 && !row(0), "scrolling replaces rows outside the viewport");

  const samples = [];
  const patchSamples = [];
  let changed = initial;
  for (let i = 0; i < 10; i++) {
    // Keep the rest of the 30,000-line document identical.
    const index = changed.indexOf(lines[25]);
    source.value = changed.slice(0, index) + "x" + changed.slice(index);
    changed = source.value;
    const start = performance.now();
    // Account for the native textarea's own layout separately.
    check(source.scrollHeight > 0, "textarea remains scrollable");
    const patchStart = performance.now();
    update();
    patchSamples.push(performance.now() - patchStart);
    samples.push(performance.now() - start);
  }
  check(!document.querySelector(".source-metrics"), "editing unwrapped text never mounts offscreen measurements");
  source.scrollTop = 0;
  syncSourceViewport();
  check(row(25).textContent.endsWith("xxxxxxxxxx" + lines[25]), "changed text is visible");
  check(!row(5).querySelector(".source-mark-active"), "stale bindings clear after edit");
  source.scrollTop = 2300;
  const oldScroll = source.scrollTop;
  source.value = "new 😀\nline\n" + changed;
  update();
  check(sourceOffsets.length === 30002 && rows.length <= 256, "multiline insertion reindexes without mounting all rows");
  check(source.scrollTop === oldScroll, "the overlay must not scroll the textarea on input");
  source.scrollTop = 0;
  syncSourceViewport();
  check(row(0).textContent.endsWith("new 😀") && row(1).textContent.endsWith("line"), "inserted rows");
  source.value = "";
  update();
  check(rows.length === 1, "deleting the entire document leaves one line");
  source.value = initial;
  update();
  scrollSourceTo(secondOffset, false);
  check(row(800).querySelector(".source-badge")?.textContent === "2", "undo restores the active mark");

  const wrapped = Array.from({length: 520}, (_, index) => {
    if (index % 64 === 63) return "";
    if (index % 7 === 0) return ("\tПеренос строки 😀 " + "длинная-ссылка-".repeat(8)).repeat(3) + "   ";
    if (index % 15 === 0) return "x".repeat(480);
    return "Row " + index;
  });
  source.value = wrapped.join("\n");
  update();
  const metrics = document.querySelector(".source-metrics");
  check(metrics.children.length === Math.ceil(wrapped.length / 64), "wrapped documents only measure one block for 64 lines");
  const wrapCheck = (width, points) => {
    byId("source-pane").style.width = width + "px";
    updateSourceLineHeight(points, 100);
    syncSourceViewport();
    const reference = element("div");
    reference.style.cssText = "position:absolute;visibility:hidden;padding-block:var(--source-padding);width:var(--source-width)";
    for (const text of wrapped) reference.append(element("div", text, "source-row"));
    byId("source-decoration").append(reference);
    check(Math.abs(source.scrollHeight - reference.offsetHeight) <= 2, "reference matches native soft wrapping at width " + width);
    check(Math.abs(reference.offsetHeight - metrics.offsetHeight) <= 2, "compact measurement matches wrapped lines at width " + width);
    for (const index of [64, 129, 320, 480]) {
      scrollSourceTo(sourceOffsets[index], false);
      check(Math.abs(source.scrollTop - reference.children[index - 1].offsetTop) <= 2,
        "jump across wrapped pages " + index + " at width " + width);
      check(Math.abs(row(index - 1).getBoundingClientRect().top - source.getBoundingClientRect().top) <= 2,
        "ruler alignment in a wrapped page " + index + " at width " + width);
    }
    reference.remove();
  };
  wrapCheck(820, 23);
  wrapCheck(395, 27);
  samples.sort((left, right) => left - right);
  patchSamples.sort((left, right) => left - right);
  byId("result").textContent = JSON.stringify({failures, initialRows, openMs, medianEditMs: samples[5], medianPatchMs: patchSamples[5]});
} catch (error) {
  byId("result").textContent = JSON.stringify({failures: [...failures, String(error)]});
}
"""


def main():
    chromium = os.environ.get("CHROME_PATH") or shutil.which("chromium") or shutil.which("google-chrome")
    if not chromium:
        candidate = Path(os.environ.get("ProgramFiles", "C:/Program Files")) / "Google/Chrome/Application/chrome.exe"
        chromium = str(candidate) if candidate.is_file() else None
    if not chromium:
        raise SystemExit("Set CHROME_PATH to your Chromium or Chrome executable.")
    with tempfile.TemporaryDirectory(prefix="pane-source-test-") as folder:
        folder = Path(folder)
        profile = folder / "profile"
        fixture = folder / "source.html"
        fixture.write_text(f"""<!doctype html><meta charset="utf-8">
<link rel="stylesheet" href="{escape((ROOT / 'app.css').as_uri())}">
<div id="source-pane" style="width: 820px; height: 580px">
  <div id="source-decoration"><div id="source-lines"></div></div>
  <textarea id="source"></textarea>
</div><output id="result"></output>
<script src="{escape((ROOT / 'components.js').as_uri())}"></script>
<script src="{escape((ROOT / 'source-pane.js').as_uri())}"></script>
<script>{SCENARIO}</script>""", encoding="utf-8")
        run = subprocess.run(
            [chromium, "--headless", "--disable-gpu", "--no-sandbox", "--disable-dev-shm-usage", f"--user-data-dir={profile}", "--dump-dom", fixture.as_uri()],
            capture_output=True, encoding="utf-8", errors="replace", timeout=90,
        )
        if run.returncode:
            raise SystemExit(f"Chromium exited {run.returncode}: {run.stderr[-3000:]}")
        result = re.search(r'<output id="result">(.*?)</output>', run.stdout, re.DOTALL)
        if not result or not result[1]:
            raise SystemExit("The Chromium fixture did not finish running.")
        report = json.loads(unescape(result[1]))
    print(json.dumps(report))
    return bool(report["failures"])


if __name__ == "__main__":
    raise SystemExit(main())
