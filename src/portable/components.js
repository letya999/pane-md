"use strict";

// Atoms used by the workspace and its dialogs.
function element(tag, text, className) {
  const item = document.createElement(tag);
  if (text !== undefined) item.textContent = text;
  if (className) item.className = className;
  return item;
}

function button(text, callback, className) {
  const item = element("button", text, className);
  item.type = "button";
  item.addEventListener("click", callback);
  return item;
}

const iconPaths = {
  left: "m10 4-4 4 4 4",
  right: "m6 4 4 4-4 4",
  outline: "M5 4h9 M5 8h6 M5 12h9 M2 4h.2 M2 8h.2 M2 12h.2",
  folder: "M2 4h5l2 2h5v7H2z",
  comments: "M3 3h10v8H8l-3 2v-2H3z",
  more: "M3 8h.2 M8 8h.2 M13 8h.2",
  plus: "M8 3v10 M3 8h10",
};

function icon(name) {
  const ns = "http://www.w3.org/2000/svg";
  const graphic = document.createElementNS(ns, "svg");
  const path = document.createElementNS(ns, "path");
  graphic.setAttribute("viewBox", "0 0 16 16");
  graphic.setAttribute("aria-hidden", "true");
  path.setAttribute("d", iconPaths[name]);
  graphic.append(path);
  return graphic;
}

function labelControl(control, label) {
  control.title = label;
  control.setAttribute("aria-label", label);
  return control;
}

function paneToggle(id, label, expanded, ...icons) {
  const control = labelControl(document.getElementById(id), label);
  control.setAttribute("aria-expanded", expanded);
  if (control.dataset.icons !== icons.join(" ")) {
    control.replaceChildren(...icons.map(icon));
    control.dataset.icons = icons.join(" ");
  }
}

function fileRow(entry, activate) {
  const row = button(undefined, activate, "pane-item file-row");
  row.dataset.file = entry.id;
  row.dataset.depth = entry.depth;
  row.style.setProperty("--depth", entry.depth);
  row.title = entry.name;
  if (entry.current) row.setAttribute("aria-current", "page");
  if (entry.expanded !== null) {
    row.setAttribute("aria-expanded", entry.expanded);
    row.append(icon("right"));
  }
  row.append(element("span", entry.name, "file-name"));
  return row;
}

function fileTreeKey(event, pane) {
  if (!event.target.matches("[data-file]") || event.altKey || event.ctrlKey ||
      event.metaKey || event.shiftKey) return;
  const rows = Array.from(pane.querySelectorAll("[data-file]"));
  const at = rows.indexOf(event.target);
  const row = rows[at];
  const depth = Number(row.dataset.depth);
  const expanded = row.getAttribute("aria-expanded");
  let next;
  switch (event.key) {
    case "ArrowUp": next = rows[at - 1]; break;
    case "ArrowDown": next = rows[at + 1]; break;
    case "Home": next = rows[0]; break;
    case "End": next = rows.at(-1); break;
    case "ArrowRight":
      if (expanded === "false") row.click();
      else if (Number(rows[at + 1]?.dataset.depth) === depth + 1) next = rows[at + 1];
      break;
    case "ArrowLeft":
      if (expanded === "true") row.click();
      else next = rows.slice(0, at).findLast((item) => Number(item.dataset.depth) === depth - 1);
      break;
    default: return;
  }
  event.preventDefault();
  next?.focus();
}

// A menu, a file tab, and the two document controls compose the toolbar.
function menu(label, items) {
  const box = element("details");
  const panel = element("div", undefined, "menu-popup");
  for (const [text, action, disabled] of items) {
    const trigger = button(text, () => {
      box.open = false;
      action();
    });
    trigger.disabled = !!disabled;
    panel.append(trigger);
  }
  box.append(element("summary", label), panel);
  return box;
}

function fileTab(tab, selected, label, onSelect, onClose) {
  const box = element("div", undefined, "tab");
  box.classList.toggle("current", selected);
  box.classList.toggle("dirty", tab.dirty);
  const open = button(tab.name, onSelect);
  open.title = tab.path ?? tab.name;
  open.setAttribute("role", "tab");
  open.setAttribute("aria-selected", selected);
  const close = labelControl(button("×", onClose, "tab-close"), label + " " + tab.name);
  box.append(open, close);
  return box;
}

function documentControls(labels, sourceView, stage, onFamily, onStage) {
  const family = element("div", undefined, "view-family");
  family.setAttribute("role", "group");
  family.setAttribute("aria-label", labels.representation);
  for (const [text, source] of [[labels.richFamily, false], [labels.sourceFamily, true]]) {
    const choice = button(text, () => {
      if (source !== sourceView) onFamily(source);
    });
    choice.setAttribute("aria-pressed", source === sourceView);
    family.append(choice);
  }
  const select = element("select");
  select.setAttribute("aria-label", labels.documentMode);
  for (const [value, text] of [["read", labels.read], ["comments", labels.comments], ["edit", labels.editing]]) {
    select.add(new Option(text, value));
  }
  select.value = stage;
  select.addEventListener("change", () => onStage(select.value));
  return [family, select];
}
