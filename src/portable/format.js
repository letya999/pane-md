"use strict";

function visualParts(includeCaret = false, includeEdges = false) {
  const selection = getSelection();
  if (!selection?.rangeCount || !includeCaret && selection.isCollapsed) return [];
  const selected = selection.getRangeAt(0);
  if (!preview.contains(selected.startContainer) || !preview.contains(selected.endContainer)) return [];
  const parts = [];
  for (const region of preview.querySelectorAll("[data-review-region]")) {
    if (!selected.intersectsNode(region) || region.closest("[hidden]")) continue;
    let position = 0;
    let start;
    let end;
    let text = "";
    for (const span of region.querySelectorAll("[data-review-run],[data-review-width]")) {
      if (span.dataset.reviewWidth !== undefined) {
        const value = span.dataset.reviewText;
        if (includeEdges && value !== undefined && selected.intersectsNode(span)) {
          if (start === undefined) start = position;
          end = position + value.length;
          text += value;
        }
        position += Number(span.dataset.reviewWidth);
        continue;
      }
      const value = span.textContent;
      if (selected.intersectsNode(span)) {
        const offset = (container, boundary, fallback) => {
          if (!span.contains(container)) return fallback;
          const range = document.createRange();
          range.selectNodeContents(span);
          range.setEnd(container, boundary);
          return range.toString().length;
        };
        const low = offset(selected.startContainer, selected.startOffset, 0);
        const high = offset(selected.endContainer, selected.endOffset, value.length);
        if (low < high ||
            includeCaret && selection.isCollapsed && span.contains(selected.startContainer) ||
            includeEdges && (!value && span.hasAttribute("data-edit") ||
              span.contains(selected.startContainer) || span.contains(selected.endContainer))) {
          if (start === undefined) start = position + low;
          end = position + high;
          text += value.slice(low, high);
        }
      }
      position += value.length;
    }
    if (start !== undefined) {
      parts.push({ region: Number(region.dataset.reviewRegion), start, end, text });
    }
  }
  return parts;
}

function editSpan(node) {
  return (node.nodeType === Node.ELEMENT_NODE ? node : node.parentElement)?.closest("[data-edit]");
}

function needsVisualReplacement() {
  const selection = getSelection();
  if (!selection?.rangeCount || selection.isCollapsed ||
      !preview.contains(selection.anchorNode) || !preview.contains(selection.focusNode)) return false;
  const start = editSpan(selection.anchorNode);
  if (!start || start !== editSpan(selection.focusNode)) return true;
  return !start.closest("pre") &&
    Boolean(start.closest("strong,em,s,code,mark,a[data-link]")) &&
    selection.toString().length === start.textContent.length;
}

function enableVisualSelection() {
  let anchor = null;
  let extendsAcrossRuns = false;
  const suspended = new Set();
  const finish = () => {
    anchor = null;
    for (const span of suspended) span.setAttribute("contenteditable", "true");
    suspended.clear();
  };
  document.addEventListener("mousedown", finish, true);
  preview.addEventListener("mousedown", (event) => {
    extendsAcrossRuns = false;
    if (event.button !== 0 || event.detail !== 1 || event.ctrlKey || event.metaKey || event.shiftKey ||
        state?.document?.mode !== 2 || state.document.reviewEditor?.draft ||
        event.target.closest("input,button")) return;
    const point = document.caretRangeFromPoint(event.clientX, event.clientY);
    const span = (point && editSpan(point.startContainer)) ??
      event.target.closest("td,th")?.querySelector("[data-edit]:empty");
    if (!span) return;
    event.preventDefault();
    anchor = span.contains(point?.startContainer)
      ? { node: point.startContainer, offset: point.startOffset }
      : { node: span, offset: 0 };
    span.focus({ preventScroll: true });
    getSelection().collapse(anchor.node, anchor.offset);
  });
  preview.addEventListener("dragstart", (event) => {
    if (anchor) event.preventDefault();
  });
  document.addEventListener("mousemove", (event) => {
    if (!anchor || !(event.buttons & 1) || !preview.contains(anchor.node)) return;
    const end = document.caretRangeFromPoint(event.clientX, event.clientY);
    if (!end || !preview.contains(end.startContainer)) return;
    const startSpan = editSpan(anchor.node);
    const endSpan = editSpan(end.startContainer);
    if (!extendsAcrossRuns) {
      if (startSpan !== endSpan) {
        extendsAcrossRuns = true;
        // Separate editing hosts clamp WebKit's native selection to the first run.
        reading.focus({ preventScroll: true });
      }
    }
    event.preventDefault();
    for (const span of [startSpan, endSpan]) {
      if (extendsAcrossRuns && span?.hasAttribute("contenteditable")) {
        span.removeAttribute("contenteditable");
        suspended.add(span);
      }
    }
    getSelection().setBaseAndExtent(
      anchor.node, anchor.offset, end.startContainer, end.startOffset,
    );
  });
  for (const type of ["mouseup", "pointercancel"]) {
    document.addEventListener(type, () => { anchor = null; });
  }
  reading.addEventListener("keydown", (event) => {
    if (!suspended.size || event.shiftKey || event.ctrlKey || event.metaKey || event.altKey ||
        !["ArrowLeft", "ArrowRight", "Escape"].includes(event.key)) return;
    const selection = getSelection();
    if (!selection.rangeCount) return;
    const range = selection.getRangeAt(0);
    const left = event.key === "ArrowLeft";
    const node = left ? range.startContainer : range.endContainer;
    const offset = left ? range.startOffset : range.endOffset;
    finish();
    editSpan(node)?.focus({ preventScroll: true });
    selection.collapse(node, offset);
    event.preventDefault();
  });
  return finish;
}

function replaceVisualSelection(text) {
  const selection = visualParts(true, true);
  if (selection.length) send("replace_visual", { selection, text });
  else status.textContent = state.labels.chooseTextBlock;
}

function formatSelection() {
  const parts = visualParts(true);
  return parts.length === 1 ? parts[0] : null;
}

function formatDocument(operation, fields = {}, selection = formatSelection()) {
  if (state.document?.mode !== 2) return;
  if (!selection) {
    status.textContent = state.labels.chooseTextBlock;
    return;
  }
  send("format", { operation, selection, ...fields });
}

function formatMenu(group) {
  const t = state.labels;
  let selected = null;
  let link = null;
  const capture = () => {
    selected = formatSelection();
    link = selectedLink();
  };
  const toggle = group.querySelector("summary");
  toggle.addEventListener("focus", capture);
  toggle.addEventListener("pointerdown", capture);
  preserveSelection(toggle);
  const panel = group.querySelector(".menu-popup");
  panel.classList.add("format-commands");
  const add = (container, label, operation, fields = {}) => {
    const trigger = preserveSelection(button(label, () => {
      group.open = false;
      if (operation === "link") showInsertLink(selected, link);
      else if (operation === "image") send("insert_image", { selection: selected });
      else formatDocument(operation, fields, selected);
    }));
    container.append(trigger);
  };
  const section = (label) => {
    const details = element("details", undefined, "format-section");
    details.append(preserveSelection(element("summary", label)));
    details.addEventListener("toggle", () => {
      if (!details.open) return;
      for (const other of panel.querySelectorAll(".format-section[open]")) {
        if (other !== details) other.open = false;
      }
    });
    const contents = element("div");
    details.append(contents);
    panel.append(details);
    return contents;
  };
  add(panel, t.bold, "bold");
  add(panel, t.italic, "italic");
  add(panel, t.strike, "strike");
  add(panel, t.code, "code");
  add(panel, t.insertLink, "link");
  const blocks = section(t.paragraphAndLists);
  add(blocks, t.paragraph, "heading", { level: 0 });
  for (let level = 1; level <= 6; level++) add(blocks, t["heading" + level], "heading", { level });
  add(blocks, t.bullet, "bullet");
  add(blocks, t.numbered, "numbered");
  add(blocks, t.task, "task");
  const table = section(t.tableCommands);
  add(table, t.insertTable, "insert_table");
  add(table, t.insertRow, "insert_row");
  add(table, t.deleteRow, "delete_row");
  add(table, t.insertColumn, "insert_column");
  add(table, t.deleteColumn, "delete_column");
  add(panel, t.insertImage, "image");
}

function selectedLink() {
  const selection = getSelection();
  if (!selection?.rangeCount) return null;
  const anchor = (node) => (node instanceof Element ? node : node?.parentElement)?.closest("a[data-link]");
  const range = selection.getRangeAt(0);
  const start = anchor(range.startContainer);
  const end = anchor(range.endContainer);
  return start && start.dataset.link === end?.dataset.link ? start.dataset.link : null;
}

function showInsertLink(selection = formatSelection(), target = selectedLink()) {
  if (!selection || selection.start === selection.end && target === null) {
    status.textContent = state.labels.selectFormatting;
    return;
  }
  const dialog = byId("insert-link");
  const form = element("form");
  const title = element("h2", target === null ? state.labels.insertLink : state.labels.editLink);
  title.id = "insert-link-title";
  const address = element("input");
  address.type = "text";
  address.required = true;
  address.spellcheck = false;
  address.value = target ?? "";
  const label = element("label", state.labels.linkAddress);
  label.append(address);
  const actions = element("div", undefined, "appearance-actions");
  const submit = element("button", state.labels.apply, "appearance-apply");
  submit.type = "submit";
  const apply = (value) => {
    dialog.close();
    formatDocument("link", { target: value }, selection);
  };
  if (target !== null) actions.append(button(state.labels.removeLink, () => apply("")));
  actions.append(button(state.labels.cancel, () => dialog.close()), submit);
  form.append(title, label, actions);
  form.addEventListener("submit", (event) => {
    event.preventDefault();
    apply(address.value);
  });
  dialog.replaceChildren(form);
  dialog.showModal();
  address.focus();
  address.select();
}

function restoreVisualCaret(location) {
  if (!location) return;
  const region = byId("r" + location.region);
  const spans = region && Array.from(region.querySelectorAll("[data-edit]"));
  if (!spans?.length) {
    if (region?.hasAttribute("tabindex")) region.focus({ preventScroll: true });
    region?.scrollIntoView({ block: "nearest" });
    return;
  }
  const point = (offset, last) => {
    const order = last ? spans : spans.slice().reverse();
    const chosen = order.find((span) => {
      const [, start, end] = span.dataset.edit.split(":").map(Number);
      return start <= offset && offset <= end;
    }) ?? spans[0];
    const [, start] = chosen.dataset.edit.split(":").map(Number);
    let remaining = Math.max(0, offset - start);
    const walk = document.createTreeWalker(chosen, NodeFilter.SHOW_TEXT);
    let tail = null;
    while (walk.nextNode()) {
      const node = walk.currentNode;
      if (remaining <= node.length) return { chosen, node, index: remaining };
      remaining -= node.length;
      tail = node;
    }
    return { chosen, node: tail ?? chosen, index: tail?.length ?? 0 };
  };
  const a = point(location.start, false);
  const b = location.start === location.end ? a : point(location.end, true);
  const range = document.createRange();
  range.setStart(a.node, a.index);
  range.setEnd(b.node, b.index);
  a.chosen.focus({ preventScroll: true });
  const selection = getSelection();
  selection.removeAllRanges();
  selection.addRange(range);
  a.chosen.scrollIntoView({ block: "nearest" });
}

function tabTableCell(event) {
  const selector = "td[data-review-region],th[data-review-region]";
  const selection = getSelection();
  const anchor = selection?.rangeCount && selection.getRangeAt(0).startContainer;
  const cell = event.target.closest(selector) ??
    (anchor instanceof Element ? anchor : anchor?.parentElement)?.closest(selector);
  if (!cell) return false;
  const row = cell.parentElement;
  const backwards = event.shiftKey;
  const neighbor = backwards
    ? cell.previousElementSibling ?? row.previousElementSibling?.lastElementChild
    : cell.nextElementSibling ?? row.nextElementSibling?.firstElementChild;
  if (!neighbor && backwards) return false;
  event.preventDefault();
  clearVisualSelection();
  if (neighbor) {
    restoreVisualCaret({ region: Number(neighbor.dataset.reviewRegion), start: 0, end: 0 });
  } else {
    formatDocument("insert_row", {}, {
      region: Number(cell.dataset.reviewRegion), start: 0, end: 0, text: "",
    });
  }
  return true;
}
