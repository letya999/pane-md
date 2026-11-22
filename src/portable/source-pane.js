"use strict";

// The textarea owns input, selection and clipboard. This layer only paints it.
const SOURCE_PAGE_LINES = 64;
let sourceRows = [];
let sourceOffsets = new Uint32Array();
let sourceMetrics;
let measuredText;
let sourceLayoutWidth = -1;
let fixedSourceLineHeight = 0;
let sourcePaddingTop = 0;
let renderedStart = -1;
let renderedEnd = -1;
let paintedText;
let paintedDocument;
let paintedFocus;
let sourceMarks = new Map();
let sourceBadge;

function indexSource(text) {
  const lines = text.split("\n");
  sourceOffsets = new Uint32Array(lines.length);
  let offset = 0;
  for (let index = 0; index < lines.length; index++) {
    sourceOffsets[index] = offset;
    offset += lines[index].length + 1;
  }
  const digits = String(lines.length).length;
  if (digits !== String(sourceRows.length).length || !sourceRows.length) {
    byId("source-pane").style.setProperty("--line-digits", digits);
  }
  sourceRows = lines;
  sourceLayoutWidth = -1;
  renderedStart = -1;
}

function measureWrappedSource(pages) {
  if (!sourceMetrics) {
    sourceMetrics = element("div", undefined, "source-metrics");
    byId("source-lines").before(sourceMetrics);
  }
  if (measuredText === paintedText) return;
  // A final break gives empty lines the same height as they have in the textarea.
  const existing = sourceMetrics.children;
  const fragment = document.createDocumentFragment();
  for (let page = 0; page < pages; page++) {
    const first = page * SOURCE_PAGE_LINES;
    const value = sourceRows.slice(first, first + SOURCE_PAGE_LINES).join("\n");
    const block = existing[page];
    if (block) {
      if (block.firstChild.data !== value) block.firstChild.data = value;
    } else {
      const block = element("div", undefined, "source-measure");
      block.append(document.createTextNode(value), element("br"));
      fragment.append(block);
    }
  }
  while (existing.length > pages) sourceMetrics.lastElementChild.remove();
  sourceMetrics.append(fragment);
  measuredText = paintedText;
}

function sourceRowAt(offset) {
  let low = 0;
  let high = sourceOffsets.length;
  while (low < high) {
    const middle = (low + high) >>> 1;
    if (sourceOffsets[middle] <= offset) low = middle + 1;
    else high = middle;
  }
  return Math.max(0, low - 1);
}

function indexSourceMarks(doc, text, focus) {
  sourceMarks = new Map();
  sourceBadge = null;
  renderedStart = -1;
  if (!doc) return;

  const marksFor = (index) => {
    if (!sourceMarks.has(index)) sourceMarks.set(index, []);
    return sourceMarks.get(index);
  };
  const add = (start, end, style) => {
    if (start >= end) return;
    const last = sourceRowAt(end - 1);
    for (let index = sourceRowAt(start); index <= last; index++) {
      const offset = sourceOffsets[index];
      const left = Math.max(start - offset, 0);
      const right = Math.min(end - offset, sourceRows[index].length);
      if (left < right) marksFor(index).push([left, style, 1], [right, style, -1]);
    }
  };
  for (const { offset } of doc.headings) {
    const marker = /^#{1,6}(?=\s|$)/.exec(text.slice(offset, offset + 7));
    if (!marker) continue;
    const row = sourceRowAt(offset);
    add(offset, offset + marker[0].length, "source-accent");
    add(offset, sourceOffsets[row] + sourceRows[row].length, "source-heading");
  }
  for (const [start, end] of doc.sourceRecords) add(start, end, "source-metadata");

  const focusedIndex = focus ? doc.reviews.findIndex(({ id }) => id === focus) : -1;
  if (doc.showReviews) {
    doc.reviews.forEach(({ location }, index) => {
      if (!location) return;
      const active = index === focusedIndex;
      add(location.offset, location.endOffset, active ? "source-mark-active" : "source-mark");
      if (active) {
        sourceBadge = { row: sourceRowAt(location.offset), number: focusedIndex + 1 };
      }
    });
  }

  for (const marks of sourceMarks.values()) marks.sort(([a], [b]) => a - b);
}

function sourceRow(index) {
  const row = element("div", undefined, "source-row");
  row.dataset.line = index + 1;
  const value = sourceRows[index];
  if (sourceBadge?.row === index) row.append(element("span", String(sourceBadge.number), "source-badge"));
  const marks = sourceMarks.get(index);
  if (!marks) {
    row.append(value);
    return row;
  }
  const styles = new Map();
  let position = 0;
  const append = (end) => {
    if (end === position) return;
    const segment = value.slice(position, end);
    row.append(styles.size ? element("span", segment, [...styles.keys()].join(" ")) : segment);
    position = end;
  };
  for (const [at, style, delta] of marks) {
    append(at);
    const count = (styles.get(style) ?? 0) + delta;
    if (count) styles.set(style, count);
    else styles.delete(style);
  }
  append(value.length);
  return row;
}

function drawSourceRows(firstPage, lastPage) {
  const start = firstPage * SOURCE_PAGE_LINES;
  const end = Math.min((lastPage + 1) * SOURCE_PAGE_LINES, sourceRows.length);
  if (start === renderedStart && end === renderedEnd) return;
  const fragment = document.createDocumentFragment();
  for (let index = start; index < end; index++) fragment.append(sourceRow(index));
  byId("source-lines").replaceChildren(fragment);
  renderedStart = start;
  renderedEnd = end;
}

function sourcePageTop(page) {
  if (fixedSourceLineHeight) return sourcePaddingTop + page * SOURCE_PAGE_LINES * fixedSourceLineHeight;
  return sourceMetrics.children[page].offsetTop;
}

function sourcePageAt(y, count) {
  if (fixedSourceLineHeight) {
    const page = Math.floor((y - sourcePaddingTop) / (SOURCE_PAGE_LINES * fixedSourceLineHeight));
    return Math.max(0, Math.min(page, count - 1));
  }
  const pages = sourceMetrics.children;
  let low = 0;
  let high = pages.length - 1;
  while (low < high) {
    const middle = (low + high) >>> 1;
    const page = pages[middle];
    if (page.offsetTop + page.offsetHeight <= y) low = middle + 1;
    else high = middle;
  }
  return low;
}

function paintSource() {
  if (source.hidden) return;
  const text = source.value;
  const doc = text === state.document.source ? state.document : null;
  const focus = doc?.showReviews ? focusedReview : null;
  if (paintedText !== text) {
    indexSource(text);
    paintedText = text;
  }
  if (paintedDocument !== doc || paintedFocus !== focus) {
    indexSourceMarks(doc, text, focus);
    paintedDocument = doc;
    paintedFocus = focus;
  }
  syncSourceViewport();
}

function syncSourceViewport() {
  if (!sourceRows.length || source.hidden) return;
  const scroll = source.scrollTop;
  const viewportHeight = source.clientHeight;
  const width = source.clientWidth + source.offsetLeft;
  const count = Math.ceil(sourceRows.length / SOURCE_PAGE_LINES);
  if (width !== sourceLayoutWidth) {
    const style = getComputedStyle(source);
    const pitch = parseFloat(style.lineHeight);
    sourcePaddingTop = parseFloat(style.paddingTop);
    const unwrappedHeight = sourceRows.length * pitch + sourcePaddingTop + parseFloat(style.paddingBottom);
    // The native control adds at least one full line height for every soft wrap.
    fixedSourceLineHeight = Math.abs(source.scrollHeight - unwrappedHeight) <= 1 ? pitch : 0;
    sourceLayoutWidth = width;
    byId("source-decoration").style.setProperty("--source-width", width + "px");
    if (fixedSourceLineHeight) {
      sourceMetrics?.remove();
      sourceMetrics = undefined;
      measuredText = undefined;
    } else {
      measureWrappedSource(count);
    }
  }
  const first = Math.max(0, sourcePageAt(scroll, count) - 1);
  const last = Math.min(count - 1, sourcePageAt(scroll + viewportHeight, count) + 1);
  const top = sourcePageTop(first);
  drawSourceRows(first, last);
  byId("source-lines").style.transform = "translateY(" + (top - scroll) + "px)";
}

function updateSourceLineHeight(halfPoints, percent) {
  // An integral line height keeps the textarea and the overlay aligned at distant rows.
  const pixels = Math.round(halfPoints * percent / 100);
  byId("source-pane").style.setProperty("--source-line-height", pixels + "px");
  sourceLayoutWidth = -1;
  if (sourceRows.length && !source.hidden) requestAnimationFrame(syncSourceViewport);
}

function scrollSourceTo(offset, keepVisible) {
  if (source.hidden) return;
  if (paintedText !== source.value) paintSource();
  else syncSourceViewport();
  const index = sourceRowAt(offset);
  const preceding = Math.max(0, index - 1);
  if (preceding < renderedStart || index >= renderedEnd) {
    drawSourceRows(Math.floor(preceding / SOURCE_PAGE_LINES), Math.floor(index / SOURCE_PAGE_LINES));
  }
  const top = sourcePageTop(renderedStart / SOURCE_PAGE_LINES);
  const rows = byId("source-lines").children;
  const row = rows[index - renderedStart];
  const start = top + row.offsetTop;
  if (!keepVisible || start < source.scrollTop ||
      start + row.offsetHeight > source.scrollTop + source.clientHeight) {
    source.scrollTop = index ? top + rows[preceding - renderedStart].offsetTop : 0;
  }
  syncSourceViewport();
}

function connectSourcePane() {
  let frame;
  source.addEventListener("scroll", syncSourceViewport);
  source.addEventListener("input", () => {
    cancelAnimationFrame(frame);
    frame = requestAnimationFrame(paintSource);
  });
  const composing = (value) => byId("source-pane").classList.toggle("composing", value);
  source.addEventListener("compositionstart", () => composing(true));
  source.addEventListener("compositionend", () => composing(false));
  new ResizeObserver(syncSourceViewport).observe(source);
}
