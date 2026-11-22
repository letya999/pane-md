"use strict";

const byId = (id) => document.getElementById(id);
const source = byId("source");
const preview = byId("preview");
const reading = byId("reading");
const status = byId("status");
const systemTheme = matchMedia("(prefers-color-scheme: dark)");
const compactWorkspace = matchMedia("(max-width: 760px)");
const views = new Map();
const requestedRemoteImages = new Set();
let state = null;
let busy = false;
let navigation = null;
let foldedNavigation = null;
let activeHeading = null;
let showReviews = true;
let zoom = 100;
let search = null;
let findHeadings = false;
let headingSearchPending = false;
let restoreHeadingFocus = false;
let paneCaret = null;
let replacement = null;
let sourceMatchesCache = null;
let restoreReplaceFocus = false;
const searchOptions = { case_sensitive: false, whole_word: false };
let lastHit = null;
let markedSearchRun = null;
let restoreSourceFocus = false;
let restoreFileFocus = null;

function isSource() {
  return (state?.document?.mode ?? 0) % 2 === 1;
}

function showRemoteImage(container) {
  const image = container.querySelector("[data-remote-image]");
  const button = container.querySelector("[data-load-remote-image]");
  if (!image || !button) return;
  const url = image.dataset.remoteImage;
  image.onload = () => { image.hidden = false; button.hidden = true; };
  image.onerror = () => {
    image.hidden = true;
    button.hidden = false;
    requestedRemoteImages.delete(url);
  };
  image.src = url;
}

function richEdits() {
  return Array.from(preview.querySelectorAll("[data-edit]"))
    .filter((span) => span.textContent !== span.dataset.original)
    .map((span) => {
      const [region, start, end] = span.dataset.edit.split(":").map(Number);
      return { region, start, end, text: span.textContent };
    });
}

function pending() {
  if (state?.document?.reviewEditor?.draft) return {};
  if (state?.document?.mode === 3 && source.value !== state.document.source) {
    return { source: source.value };
  }
  if (state?.document?.mode === 2) return { edits: richEdits() };
  return {};
}

function saveView() {
  if (!state?.document) return;
  const tab = state.tabs[state.active];
  const node = isSource() ? source : reading;
  const part = state.document.mode === 2 ? formatSelection() : null;
  const view = {
    scrollTop: node.scrollTop,
    scrollLeft: node.scrollLeft,
    selection: [source.selectionStart, source.selectionEnd],
    visual: part && { region: part.region, start: part.start, end: part.end },
  };
  views.set(tab.id + ":" + isSource(), view);
  return view;
}

function restoreView(previous) {
  if (!state.document) return;
  const view = previous ?? views.get(state.tabs[state.active].id + ":" + isSource());
  const node = isSource() ? source : reading;
  if (isSource()) source.setSelectionRange(...(view?.selection ?? [0, 0]));
  node.scrollTop = view?.scrollTop ?? 0;
  node.scrollLeft = view?.scrollLeft ?? 0;
  return view;
}

function linkView(view) {
  if (!view || isSource()) return view;
  const point = (node, offset) => {
    const block = (node instanceof Element ? node : node.parentElement)?.closest("[data-review-region]");
    if (!block || !preview.contains(block)) return null;
    const before = document.createRange();
    before.selectNodeContents(block);
    before.setEnd(node, offset);
    return [Number(block.dataset.reviewRegion), before.toString().length];
  };
  const selection = getSelection();
  const range = selection?.rangeCount ? selection.getRangeAt(0) : null;
  const start = range && point(range.startContainer, range.startOffset);
  const end = range && point(range.endContainer, range.endOffset);
  const marked = preview.querySelector(".jump-target");
  return {
    ...view,
    visualSelection: start && end ? [start, end] : null,
    jumpRegion: marked ? Number(marked.id.slice(1)) : null,
  };
}

function restoreLinkView(view) {
  if (!view || isSource()) return;
  if (view.jumpRegion != null) byId("r" + view.jumpRegion)?.classList.add("jump-target");
  const point = ([region, offset]) => {
    const block = preview.querySelector(`[data-review-region="${region}"]`);
    if (!block || block.closest("[hidden]")) return null;
    const walker = document.createTreeWalker(block, NodeFilter.SHOW_TEXT);
    while (walker.nextNode()) {
      const node = walker.currentNode;
      if (offset <= node.length) return [node, offset];
      offset -= node.length;
    }
    return null;
  };
  if (view.visualSelection) {
    const [start, end] = view.visualSelection.map(point);
    if (start && end) {
      start[0].parentElement.closest("[contenteditable]")?.focus({ preventScroll: true });
      const range = document.createRange();
      range.setStart(...start);
      range.setEnd(...end);
      const selection = getSelection();
      selection.removeAllRanges();
      selection.addRange(range);
    }
  }
  reading.scrollTop = view.scrollTop;
  reading.scrollLeft = view.scrollLeft;
}

function setBusy(value) {
  busy = value;
  byId("menu").inert = value;
  byId("workspace").inert = value;
  byId("document-bar").inert = value;
}

function send(command, fields = {}) {
  if (busy) return;
  const viewport = saveView();
  const changes = pending();
  restoreSourceFocus = document.activeElement === source;
  if (command === "refresh_files") {
    restoreFileFocus = document.activeElement.dataset.file;
  }
  if (["link", "source_link", "link_history"].includes(command)) {
    fields = { ...fields, viewport: linkView(viewport) };
  }
  setBusy(true);
  window.ipc.postMessage(JSON.stringify({ command, ...fields, ...changes }));
}

function cycleTab(direction) {
  if (state?.tabs.length > 1 && !state.document?.reviewEditor?.draft && !document.querySelector("dialog[open]")) {
    const count = state.tabs.length;
    send("select_tab", { index: (state.active + count + direction) % count });
  }
}

function renderMenu() {
  const t = state.labels;
  const noDocument = !state.document;
  const historyLabel = (text, shortcut) => state.macos ? text.split("\t")[0] + "\t" + shortcut : text;
  for (const old of byId("menu").querySelectorAll("details")) old.remove();
  const editMenu = menu(t.edit, [
    [t.undo, () => history(false), noDocument],
    [t.redo, () => history(true), noDocument],
    [t.find, showFind, noDocument],
    ...(state.document?.mode === 3 ? [[t.showReplace, () => showFind(true)]] : []),
    [t.lint, () => moveLint(false), noDocument],
    [t.addReview, addReview, !state.document?.showReviews],
  ]);
  const formatting = menu(t.format, []);
  if (state.document?.mode === 2 && !state.document.reviewEditor?.draft) {
    formatMenu(formatting);
  } else {
    formatting.querySelector(".menu-popup").append(element("div", t.formatInVisualEdit, "panel-note"));
  }
  const viewMenu = preserveSelection(menu(t.view, [
    [t.findHeadingMenu, () => showFind(false, true), noDocument],
    [t.showSource, showInSource, noDocument || isSource()],
    [t.outline, () => setNavigation(navigation === "outline" ? null : "outline"), noDocument],
    [t.files, () => setNavigation(navigation === "folder" ? null : "folder"), !state.folder],
    [t.refreshFiles, () => send("refresh_files"), !state.folder],
    [t.reviews, () => setReviews(!showReviews), noDocument || !state.document?.showReviews],
    [historyLabel(t.back, "⌘+["), () => send("link_history", { forward: false }), !state.canGoBack],
    [historyLabel(t.forward, "⌘+]"), () => send("link_history", { forward: true }), !state.canGoForward],
  ]));
  byId("menu").append(
    menu(t.file, [
      [t.newNote, () => send("new_note"), state.wait],
      [t.open, () => send("open_file"), state.wait],
      [t.folder, () => send("open_folder"), state.wait],
      [t.save, () => send("save"), noDocument],
      [t.saveAs, () => send("save_as"), state.wait || noDocument],
      [t.close, () => send("close_tab", { index: state.active }), noDocument],
      [t.quit, () => send("exit")],
    ]),
    editMenu,
    viewMenu,
    formatting,
    menu(t.settings, [
      [t.appearance, showEditorAppearance],
      [t.light, () => send("theme", { theme: "light" })],
      [t.dark, () => send("theme", { theme: "dark" })],
      [t.system, () => send("theme", { theme: "system" })],
      [t.ru, () => send("language", { english: false })],
      [t.en, () => send("language", { english: true })],
    ]),
  );
}

function renderTabs() {
  const tabs = byId("tabs");
  tabs.replaceChildren();
  for (const [index, tab] of state.tabs.entries()) {
    const selected = index === state.active;
    tabs.append(fileTab(tab, selected, state.labels.close, () => {
      if (!selected) send("select_tab", { index });
    }, () => send("close_tab", { index })));
  }
  if (!state.wait) {
    const add = labelControl(button(undefined, () => send("open_file"), "open-tab"), state.labels.openTab);
    add.append(icon("plus"));
    tabs.append(add);
  }
}

function renderDocumentControls() {
  const modes = byId("modes");
  const doc = state.document;
  modes.hidden = !doc;
  modes.replaceChildren();
  if (!doc) return;
  const stage = doc.mode >= 2 ? "edit" : doc.showReviews ? "comments" : "read";
  modes.append(...documentControls(state.labels, isSource(), stage,
    (sourceView) => send("family", { source_view: sourceView }),
    (value) => send("stage", { stage: value })));
  modes.inert = !!doc.reviewEditor?.draft;
}

function jump(location, focus = true, keepVisible = false) {
  if (!location) return;
  if (isSource()) {
    const start = Math.min(location.offset ?? 0, source.value.length);
    if (focus) source.focus({ preventScroll: true });
    source.setSelectionRange(start, location.endOffset ?? start);
    scrollSourceTo(start, keepVisible);
  } else {
    const target = location.region == null ? null : byId("r" + location.region);
    if (target) {
      target.scrollIntoView({ block: "center" });
      reading.querySelector(".jump-target")?.classList.remove("jump-target");
      target.classList.add("jump-target");
    } else {
      reading.scrollTop = 0;
    }
  }
}

function regionInView(preferJump = false) {
  const bounds = reading.getBoundingClientRect();
  const visible = (block) => {
    if (!block || block.closest("[hidden]")) return false;
    const rect = block.getBoundingClientRect();
    return rect.top < bounds.bottom && rect.bottom > bounds.top;
  };
  const jumped = preferJump && preview.querySelector(".jump-target");
  if (visible(jumped)) return Number(jumped.id.slice(1));
  const anchor = getSelection()?.focusNode;
  const block = (anchor instanceof Element ? anchor : anchor?.parentElement)?.closest("[data-review-region]");
  if (visible(block) && preview.contains(block)) return Number(block.dataset.reviewRegion);
  for (const candidate of preview.querySelectorAll("[data-review-region], figure[data-diagram]")) {
    if (visible(candidate)) return Number(candidate.dataset.reviewRegion ?? candidate.dataset.diagram);
  }
  return null;
}

function showInSource() {
  const parts = visualParts(state.document.mode === 2);
  send("show_in_source", {
    region: parts[0]?.region ?? regionInView(true),
    selection: parts.length ? [parts[0], parts.at(-1)] : null,
  });
}

function moveLint(previous) {
  const region = isSource() ? null : regionInView(true);
  const jump = state.document.jump;
  const caret = isSource() ? source.selectionStart : jump?.region === region ? jump.offset : null;
  send("lint", { previous, caret, region });
}

function findHeading(previous) {
  headingSearchPending = true;
  restoreHeadingFocus = document.activeElement === search;
  send("find_heading", {
    query: search.value,
    previous,
    caret: source.selectionStart,
    region: isSource() ? null : regionInView(true),
  });
}

function cyclePaneFocus(previous) {
  const documentPane = byId("document-area");
  const nav = byId("navigation");
  const reviews = byId("reviews");
  const editor = isSource() ? source : reading;
  const panes = [
    [documentPane, editor.hidden ? null : editor],
    [nav, nav.hidden ? null : nav.querySelector("[aria-current]") ??
      nav.querySelector(".pane-item") ?? byId("hide-navigation")],
    [reviews, reviews.hidden ? null : reviews.querySelector('.review-card-body[aria-pressed="true"]') ??
      reviews.querySelector(".review-card-body") ?? byId("hide-reviews")],
  ].filter(([, target]) => target);
  if (!panes.length) return;
  const current = panes.findIndex(([pane]) => pane.contains(document.activeElement));
  if (state.document?.mode === 2 && reading.contains(document.activeElement)) {
    paneCaret = { document: state.document, selection: formatSelection() };
  }
  const next = current < 0 ? (previous ? panes.length - 1 : 0) :
    (current + panes.length + (previous ? -1 : 1)) % panes.length;
  const target = panes[next][1];
  target.focus({ preventScroll: true });
  if (target === reading && paneCaret?.document === state.document) {
    restoreVisualCaret(paneCaret.selection);
  }
}

function setNavigation(value) {
  if (state.document?.reviewEditor?.draft) return;
  if (!value && navigation) foldedNavigation = navigation;
  navigation = value;
  if (value && compactWorkspace.matches && state.document?.showReviews && showReviews) {
    showReviews = false;
    renderReviews();
  }
  renderNavigation();
}

function navigationToOpen() {
  if (state.folder && (navigation ?? foldedNavigation) === "folder") return "folder";
  return state.document ? "outline" : "folder";
}

function setReviews(value) {
  if (state.document?.reviewEditor?.draft) return;
  showReviews = value;
  renderNavigation();
  renderReviews();
}

function renderNavigation() {
  const t = state.labels;
  const doc = state.document;
  const nav = byId("navigation-content");
  const canNavigate = !!doc || !!state.folder;
  const navOpen = (!(compactWorkspace.matches && doc?.showReviews && showReviews)) &&
    ((navigation === "outline" && !!doc) || (navigation === "folder" && !!state.folder));
  byId("navigation").hidden = !navOpen;
  byId("navigation-edge").hidden = !canNavigate || navOpen;
  byId("open-navigation").disabled = byId("hide-navigation").disabled = !!doc?.reviewEditor?.draft;
  const reopenFiles = navigationToOpen() === "folder";
  paneToggle("open-navigation", reopenFiles ? t.files : t.showOutline, false,
    reopenFiles ? "folder" : "outline", "right");
  paneToggle("hide-navigation", navigation === "folder" ? t.hideFiles : t.hideOutline, true, "left");
  const folderScroll = nav.dataset.folder === state.folder ? nav.scrollTop : 0;
  const fileFocus = restoreFileFocus ?? document.activeElement.dataset.file;
  restoreFileFocus = null;
  delete nav.dataset.folder;
  nav.replaceChildren();
  if (navigation === "outline" && doc) {
    byId("navigation-title").textContent = t.outline;
    const select = (item) => {
      nav.querySelector('[aria-current="location"]')?.removeAttribute("aria-current");
      item?.setAttribute("aria-current", "location");
    };
    for (const heading of doc.headings) {
      const item = button(heading.text, () => {
        activeHeading = heading.region;
        select(item);
        jump(heading);
      }, "pane-item");
      item.dataset.region = heading.region;
      item.style.paddingInlineStart = (8 + (heading.level - 1) * 10) + "px";
      item.title = heading.text;
      nav.append(item);
    }
    select(nav.querySelector(`[data-region="${activeHeading}"]`) ?? nav.firstElementChild);
  }
  if (navigation === "folder" && state.folder) {
    nav.dataset.folder = state.folder;
    byId("navigation-title").textContent = t.files;
    const path = element("div", state.folder.split("/").at(-1) || "/", "panel-note");
    path.title = state.folder;
    const up = button("↑ " + t.up, () => send("folder_up"), "pane-item");
    up.disabled = !state.folderHasParent;
    nav.append(up, path);
    for (const [index, entry] of state.entries.entries()) {
      nav.append(fileRow(entry, () => {
        if (entry.expanded !== null) restoreFileFocus = entry.id;
        send("browse", { index });
      }));
    }
    nav.scrollTop = folderScroll;
    if (fileFocus && navOpen) {
      const row = nav.querySelector(`[data-file="${fileFocus}"]`) ??
        nav.querySelector('[aria-current="page"]') ?? nav.querySelector("[data-file]");
      (row ?? byId("hide-navigation")).focus({ preventScroll: true });
    }
  }
}

const navigationContent = byId("navigation-content");
navigationContent.addEventListener("keydown", (event) => fileTreeKey(event, navigationContent));

function updateDiagrams(diagrams) {
  if (!diagrams || state?.document?.diagrams.revision !== diagrams.revision) return;
  for (const item of diagrams.entries) {
    const figure = byId("r" + item.region);
    if (!figure?.hasAttribute("data-diagram")) continue;
    const code = figure.querySelector("[data-diagram-source]");
    const image = figure.querySelector("[data-diagram-image]");
    const status = figure.querySelector("[data-diagram-status]");
    const url = item.url ?? "";
    if (image.getAttribute("src") === url) continue;
    const showPicture = (ready, message) => {
      image.hidden = !ready;
      code.hidden = ready;
      status.hidden = ready;
      if (!ready) status.textContent = message;
    };
    showPicture(false, item.state === "fallback" ? status.dataset.fallback : status.dataset.pending);
    image.onload = null;
    image.onerror = null;
    if (url) {
      image.width = item.width;
      image.height = item.height;
      image.onload = () => showPicture(true);
      image.onerror = () => showPicture(false, status.dataset.fallback);
      image.src = url;
    } else {
      image.removeAttribute("src");
    }
  }
}

function render(value) {
  const previousFolder = state?.folder;
  const hadSourceFocus = restoreSourceFocus || document.activeElement === source;
  restoreSourceFocus = false;
  const previousDocument = state?.tabs[state.active]?.id;
  const wasSource = isSource();
  if (state && value.tabs.length < state.tabs.length) {
    const remaining = new Set(value.tabs.map((tab) => tab.id));
    for (const { id } of state.tabs) {
      if (!remaining.has(id)) {
        views.delete(id + ":false");
        views.delete(id + ":true");
      }
    }
  }
  state = value;
  paneCaret = null;
  const headingJump = headingSearchPending ? value.document?.jump : null;
  headingSearchPending = false;
  if (previousDocument !== state.tabs[state.active]?.id) activeHeading = null;
  if (headingJump?.region != null) activeHeading = headingJump.region;
  if (state.folder && previousFolder !== state.folder) navigation = "folder";
  clearFindMarks();
  document.documentElement.lang = value.english ? "en" : "ru";
  document.documentElement.dataset.theme = value.theme;
  applyEditorAppearance();
  renderMenu();
  renderTabs();
  renderDocumentControls();
  labelControl(source, value.labels.sourceEditor);
  labelControl(byId("zoom-in"), value.labels.zoomIn);
  labelControl(byId("zoom-out"), value.labels.zoomOut);
  labelControl(byId("zoom-reset"), value.labels.zoomReset);
  byId("empty").hidden = !!value.document;
  byId("empty").textContent = value.labels.empty;
  byId("source-pane").hidden = source.hidden = !value.document || !isSource();
  reading.hidden = !value.document || isSource();
  const draft = value.document?.reviewEditor?.draft;
  source.readOnly = value.document?.mode !== 3 || !!draft;
  source.value = value.document?.source ?? "";
  clearVisualSelection();
  // Only Rust-generated markup from the Markdown view model reaches this element.
  preview.innerHTML = value.document?.html ?? "";
  for (const image of preview.querySelectorAll("[data-remote-image]")) {
    if (requestedRemoteImages.has(image.dataset.remoteImage)) showRemoteImage(image.closest(".remote-image"));
  }
  updateDiagrams(value.document?.diagrams);
  for (const span of preview.querySelectorAll("[data-edit]")) span.dataset.original = span.textContent;
  lastHit = null;
  if (!value.document) hideFind();
  if (value.document?.mode !== 3 || draft) byId("replace").hidden = true;
  if (search && isSource()) showSearchCount(null, sourceMatches().length);
  setBusy(false);
  renderNavigation();
  renderReviews();
  status.textContent = value.notice;
  const view = restoreView(value.linkViewport);
  syncSourceViewport();
  if (value.linkViewport) {
    (isSource() ? source : reading).focus({ preventScroll: true });
  } else if (!draft && !source.hidden && (hadSourceFocus || !wasSource || previousDocument !== state.tabs[state.active]?.id)) {
    source.focus({ preventScroll: true });
  }
  jump(value.document?.jump, !restoreHeadingFocus);
  if (restoreHeadingFocus) {
    restoreHeadingFocus = false;
    search?.focus({ preventScroll: true });
  }
  if (restoreReplaceFocus) {
    restoreReplaceFocus = false;
    if (!byId("replace").hidden) replacement?.focus({ preventScroll: true });
  }
  const editing = value.document?.mode === 2 && !draft && !value.document.jump;
  if (value.linkViewport) {
    restoreLinkView(value.linkViewport);
  } else {
    restoreVisualCaret(value.document?.editorSelection ?? (editing ? view?.visual : null));
  }
  if (draft) {
    const input = byId("review-draft-input");
    input?.closest(".review-card")?.scrollIntoView({ block: "nearest" });
    input?.focus();
    input?.setSelectionRange(input.value.length, input.value.length);
  }
}

function showFind(replacing = false, headings = false) {
  if (!state.document || replacing && state.document.mode !== 3) return;
  if (search && headings !== findHeadings) hideFind();
  findHeadings = headings;
  const t = state.labels;
  if (!search) {
    const bar = byId("find");
    search = element("input");
    search.type = "search";
    search.placeholder = headings ? t.findHeading : t.find;
    search.setAttribute("aria-label", search.placeholder);
    const changed = () => {
      lastHit = null;
      sourceMatchesCache = null;
      if (!findHeadings) find(false);
    };
    search.addEventListener("input", changed);
    const option = (text, key) => {
      const label = element("label", undefined, "find-option");
      const field = element("input");
      field.type = "checkbox";
      field.checked = searchOptions[key];
      field.addEventListener("change", () => {
        searchOptions[key] = field.checked;
        changed();
      });
      label.append(field, element("span", text));
      return label;
    };
    const control = (symbol, title, action) => {
      const trigger = button(symbol, action);
      trigger.setAttribute("aria-label", title);
      trigger.title = title;
      return trigger;
    };
    const count = element("output");
    count.id = "find-count";
    count.setAttribute("aria-live", "polite");
    bar.replaceChildren(
      search, count,
      control("↑", t.findPrevious, () => find(true)),
      control("↓", t.findNext, () => find(false)),
      control("×", t.close, hideFind),
      ...(headings ? [] : [option(t.matchCase, "case_sensitive"), option(t.wholeWord, "whole_word")]),
    );
    bar.hidden = false;
    const selected = isSource() ? source.value.slice(source.selectionStart, source.selectionEnd) : "";
    if (!headings && selected && selected.length <= 512 && !selected.includes("\n")) {
      search.value = selected;
    }
  }
  const replaceBar = byId("replace");
  if (replacing && !replacement) {
    replacement = element("input");
    replacement.type = "text";
    const label = t.replacement;
    replacement.placeholder = label;
    replacement.setAttribute("aria-label", label);
    replaceBar.replaceChildren(
      replacement,
      button(t.replaceOne, () => replaceSource(false)),
      button(t.replaceAll, () => replaceSource(true)),
    );
  }
  replaceBar.hidden = !replacing;
  search.focus();
  search.select();
}

function replaceSource(all) {
  if (state.document?.mode !== 3 || !search?.value || !replacement || byId("replace").hidden) return;
  restoreReplaceFocus = document.activeElement === replacement;
  send("replace_source", {
    query: search.value,
    replacement: replacement.value,
    options: searchOptions,
    selection: all ? null : [source.selectionStart, source.selectionEnd],
  });
}

function hideFind() {
  const range = lastHit?.range;
  const headings = findHeadings;
  clearFindMarks();
  byId("find").hidden = true;
  byId("replace").hidden = true;
  search = null;
  findHeadings = false;
  replacement = null;
  sourceMatchesCache = null;
  lastHit = null;
  if (state?.document && isSource()) source.focus();
  else if (range && preview.contains(range.startContainer) && preview.contains(range.endContainer)) {
    reading.focus({ preventScroll: true });
    getSelection().removeAllRanges();
    getSelection().addRange(range);
  } else if (headings && !reading.hidden) reading.focus({ preventScroll: true });
}

function clearFindMarks() {
  globalThis.CSS?.highlights?.delete("document-search");
  markedSearchRun?.classList.remove("search-result");
  markedSearchRun = null;
}

function searchPattern(query) {
  const literal = Array.from(query, (char) => "\\.^$*+?()[]{}|".includes(char) ? "\\" + char : char).join("");
  return new RegExp(literal, searchOptions.case_sensitive ? "gu" : "giu");
}

function wholeWord(text, start, end) {
  return !searchOptions.whole_word || (
    !/[\p{Alphabetic}\p{Number}_]$/u.test(text.slice(Math.max(0, start - 2), start)) &&
    !/^[\p{Alphabetic}\p{Number}_]/u.test(text.slice(end, end + 2))
  );
}

function sourceMatches() {
  const query = search?.value;
  if (!query) return [];
  const text = source.value;
  const flags = +searchOptions.case_sensitive + 2 * +searchOptions.whole_word;
  if (sourceMatchesCache?.text === text && sourceMatchesCache.query === query && sourceMatchesCache.flags === flags) {
    return sourceMatchesCache.ranges;
  }
  const ranges = [];
  for (const match of text.matchAll(searchPattern(query))) {
    const start = match.index;
    const end = start + match[0].length;
    if (wholeWord(text, start, end)) ranges.push({ start, end });
  }
  sourceMatchesCache = { text, query, flags, ranges };
  return ranges;
}

function showSearchCount(index, total) {
  const output = byId("find-count");
  if (!output || !search || findHeadings) return;
  output.textContent = search.value ? state.labels.searchCount
    .replace("{current}", index === null ? "0" : String(index + 1))
    .replace("{total}", String(total)) : "";
}

function find(previous) {
  if (findHeadings) {
    findHeading(previous);
    return;
  }
  clearFindMarks();
  const query = search?.value;
  if (!query) {
    showSearchCount(null, 0);
    return;
  }
  if (isSource()) {
    const ranges = sourceMatches();
    if (ranges.length) {
      const last = lastHit?.ranges === ranges ? lastHit.index : null;
      const adjacent = last === null ? (previous ? ranges.length - 1 : 0) : last + (previous ? -1 : 1);
      const index = (adjacent + ranges.length) % ranges.length;
      const match = ranges[index];
      jump({ offset: match.start, endOffset: match.end }, false);
      lastHit = { ranges, index };
      showSearchCount(index, ranges.length);
      if (status.textContent === state.labels.noMatch) status.textContent = "";
      return;
    }
  } else {
    const pattern = searchPattern(query);
    const matches = [];
    for (const block of preview.querySelectorAll("[data-review-region]")) {
      if (block.closest("[hidden]")) continue;
      const walker = document.createTreeWalker(block, NodeFilter.SHOW_TEXT);
      const nodes = [];
      let text = "";
      while (walker.nextNode()) {
        const node = walker.currentNode;
        if (node.parentElement.closest("[hidden]")) continue;
        nodes.push({ node, start: text.length });
        text += node.textContent;
      }
      for (const match of text.matchAll(pattern)) {
        if (wholeWord(text, match.index, match.index + match[0].length)) {
          matches.push({ start: match.index, end: match.index + match[0].length, nodes });
        }
      }
    }
    if (matches.length) {
      const index = lastHit?.index ?? (previous ? 0 : -1);
      const next = (index + (previous ? -1 : 1) + matches.length) % matches.length;
      const { start, end, nodes } = matches[next];
      const first = nodes.findLast(({ start: at }) => at <= start);
      const last = nodes.findLast(({ start: at }) => at < end);
      const item = first.node;
      item.parentElement.scrollIntoView({ block: "center" });
      const selected = document.createRange();
      selected.setStart(item, start - first.start);
      selected.setEnd(last.node, end - last.start);
      if (document.activeElement === search) {
        // Selecting inside contenteditable transfers focus away from the find field in WebKit.
        if (globalThis.Highlight && globalThis.CSS?.highlights) {
          CSS.highlights.set("document-search", new Highlight(selected));
        } else {
          markedSearchRun = item.parentElement;
          markedSearchRun.classList.add("search-result");
        }
      } else {
        getSelection().removeAllRanges();
        getSelection().addRange(selected);
      }
      lastHit = { index: next, range: selected.cloneRange() };
      showSearchCount(next, matches.length);
      if (status.textContent === state.labels.noMatch) status.textContent = "";
      return;
    }
  }
  showSearchCount(null, 0);
  status.textContent = state.labels.noMatch;
}

function history(redo) {
  if (state.document?.mode >= 2 || state.document?.showReviews) send("undo", { redo });
}

function adjustZoom(change) {
  zoom = change ? Math.max(60, Math.min(250, zoom + change)) : 100;
  document.documentElement.style.setProperty("--size", zoom / 100);
  if (state?.appearance) updateSourceLineHeight(state.appearance.settings.source.half_points, zoom);
  byId("zoom-reset").textContent = zoom + "%";
}

function editMarker() {
  document.querySelector(".tab.current")?.classList.add("dirty");
}

byId("hide-navigation").addEventListener("click", () => {
  setNavigation(null);
  byId("open-navigation").focus();
});
byId("open-navigation").addEventListener("click", () => {
  setNavigation(navigationToOpen());
  byId("hide-navigation").focus();
});
byId("hide-reviews").addEventListener("click", () => {
  setReviews(false);
  byId("open-reviews").focus();
});
byId("open-reviews").addEventListener("click", () => {
  setReviews(true);
  byId("hide-reviews").focus();
});
connectSourcePane();
compactWorkspace.addEventListener("change", () => {
  if (state) renderNavigation();
});
byId("zoom-in").addEventListener("click", () => adjustZoom(10));
byId("zoom-out").addEventListener("click", () => adjustZoom(-10));
byId("zoom-reset").addEventListener("click", () => adjustZoom(0));
source.addEventListener("input", () => {
  editMarker();
  if (search) {
    sourceMatchesCache = null;
    lastHit = null;
    byId("find-count").textContent = "";
  }
});
source.addEventListener("click", (event) => {
  if (event.ctrlKey || event.metaKey) send("source_link", { caret: source.selectionStart });
});
preview.addEventListener("input", editMarker);
const clearVisualSelection = enableVisualSelection();
preview.addEventListener("beforeinput", (event) => {
  const span = event.target.closest("[data-edit]");
  const selection = getSelection();
  if (state?.document?.mode === 2 && !event.isComposing && needsVisualReplacement()) {
    if (event.inputType === "insertText" && event.data !== null || event.inputType.startsWith("deleteContent")) {
      event.preventDefault();
      replaceVisualSelection(event.data ?? "");
      return;
    }
  }
  if (!span || !span.contains(selection.anchorNode) || !span.contains(selection.focusNode) ||
      /insertParagraph|insertLineBreak|insertFromDrop|insertFromPaste|format/i.test(event.inputType)) {
    event.preventDefault();
  }
});
reading.addEventListener("paste", (event) => {
  event.preventDefault();
  if (state?.document?.mode !== 2 || state.document.reviewEditor?.draft) return;
  const text = event.clipboardData?.getData("text/plain");
  if (!text) return;
  replaceVisualSelection(text);
});
preview.addEventListener("click", (event) => {
  const remote = event.target.closest("[data-load-remote-image]");
  if (remote) {
    event.preventDefault();
    requestedRemoteImages.add(remote.dataset.loadRemoteImage);
    showRemoteImage(remote.closest(".remote-image"));
    return;
  }
  const diagram = event.target.closest("[data-show-diagram-source]");
  if (diagram) {
    send("diagram_source", { region: Number(diagram.dataset.showDiagramSource) });
    return;
  }
  const link = event.target.closest("[data-link]");
  if (link) {
    event.preventDefault();
    if (state.document.mode < 2 || event.ctrlKey || event.metaKey) send("link", { target: link.dataset.link });
  } else {
    reviewMarkedClick(event);
  }
});
preview.addEventListener("change", (event) => {
  if (event.target.dataset.task !== undefined) send("toggle_task", { region: Number(event.target.dataset.task) });
});
document.addEventListener("click", (event) => {
  document.querySelectorAll("#menu > details[open], .review-more[open]").forEach((node) => {
    if (!node.contains(event.target)) node.open = false;
  });
});

document.addEventListener("keydown", (event) => {
  if (!state || busy || event.isComposing || document.querySelector("dialog[open]")) return;
  const command = event.ctrlKey || event.metaKey;
  const key = event.key.toLowerCase();
  const withDoc = !!state.document;
  const reviewAction = reviewKey(event, key);
  if (reviewAction) {
    event.preventDefault();
    reviewAction();
    return;
  }
  if (state.document?.reviewEditor?.draft) return;
  let action;
  if (event.key === "Escape" && search) action = hideFind;
  if (event.key === "F3" || event.key === "Enter" && event.target === search) action = () => find(event.shiftKey);
  if (event.key === "Enter" && replacement && event.target === replacement) action = () => replaceSource(command);
  if (event.key === "F5" && !command && !event.altKey && !event.shiftKey && state.folder) {
    action = () => send("refresh_files");
  }
  if (event.key === "F6" && !command && !event.altKey &&
      (withDoc || !byId("navigation").hidden)) action = () => cyclePaneFocus(event.shiftKey);
  if (event.key === "F8" && withDoc) action = () => moveLint(event.shiftKey);
  if (state.macos && event.metaKey && !event.ctrlKey && !event.altKey && !event.shiftKey) {
    if (event.code === "BracketLeft" && state.canGoBack) action = () => send("link_history", { forward: false });
    if (event.code === "BracketRight" && state.canGoForward) action = () => send("link_history", { forward: true });
  } else if (!state.macos && event.altKey && !command && !event.shiftKey) {
    if (event.key === "ArrowLeft" && state.canGoBack) action = () => send("link_history", { forward: false });
    if (event.key === "ArrowRight" && state.canGoForward) action = () => send("link_history", { forward: true });
  }
  const editing = state.document?.mode === 2;
  if (editing && (reading.contains(event.target) || event.target === document.body) && needsVisualReplacement()) {
    if (!event.altKey && (key === "backspace" || key === "delete")) {
      action = () => replaceVisualSelection("");
    } else if (!command && !event.altKey && Array.from(event.key).length === 1) {
      action = () => replaceVisualSelection(event.key);
    } else if (command && !event.altKey && key === "x") {
      action = () => {
        if (document.execCommand("copy")) replaceVisualSelection("");
      };
    }
  }
  if (editing && reading.contains(event.target)) {
    if (!command && !event.altKey && key === "tab" && tabTableCell(event)) return;
    if (!command && !event.altKey && key === "enter" && event.target.closest("[data-edit]")) {
      action = () => formatDocument("break", { soft: event.shiftKey });
    }
    if (command) {
      if (event.altKey && /^[0-6]$/.test(key)) action = () => formatDocument("heading", { level: Number(key) });
      else if (key === "b") action = () => formatDocument("bold");
      else if (key === "i") action = () => formatDocument("italic");
      else if (key === "k" && event.shiftKey) action = () => send("insert_image", { selection: formatSelection() });
      else if (key === "k") action = () => showInsertLink();
      else if (key === "l" && event.shiftKey) action = () => formatDocument("task");
      else if (key === " ") action = () => formatDocument("toggle_task");
    }
  }
  if (command && !action) {
    switch (key) {
      case "n": if (!state.wait && !event.altKey && !event.shiftKey) action = () => send("new_note"); break;
      case "o": action = () => send(event.shiftKey ? "open_folder" : "open_file"); break;
      case "s": if (withDoc) action = () => send(event.shiftKey ? "save_as" : "save"); break;
      case "f": if (withDoc) action = () => showFind(false); break;
      case "h": if (event.ctrlKey && !event.altKey && !event.shiftKey && state.document?.mode === 3) action = () => showFind(true); break;
      case "w": if (withDoc) action = () => send("close_tab", { index: state.active }); break;
      case "z":
      case "y": {
        if (event.target === search || event.target === replacement) break;
        const changes = pending();
        if (!changes.source && !changes.edits?.length) action = () => history(key === "y" || event.shiftKey);
        break;
      }
      case "tab":
        if (state.tabs.length > 1) action = () => cycleTab(event.shiftKey ? -1 : 1);
        break;
      case "enter": {
        if (withDoc && isSource()) {
          action = () => send("source_link", { caret: source.selectionStart });
        } else {
          const link = event.target.closest?.("[data-link]") ??
            getSelection()?.focusNode?.parentElement?.closest("[data-link]");
          if (link && preview.contains(link)) action = () => send("link", { target: link.dataset.link });
        }
        break;
      }
      case "1": case "2": case "3": case "4":
        if (withDoc) action = () => send("quick_mode", { mode: Number(key) - 1 });
        break;
      case "+": case "=": action = () => adjustZoom(10); break;
      case "-": action = () => adjustZoom(-10); break;
      case "0": action = () => adjustZoom(0); break;
    }
  }
  if (action) {
    event.preventDefault();
    action();
  }
});

window.render = render;
window.selectPreviousTab = () => cycleTab(-1);
window.updateDiagrams = updateDiagrams;
window.requestClose = () => send("exit");
window.acceptDrop = () => send("open_dropped");
window.showError = (message) => {
  headingSearchPending = false;
  restoreHeadingFocus = false;
  setBusy(false);
  if (restoreFileFocus) {
    byId("navigation-content").querySelector(`[data-file="${restoreFileFocus}"]`)?.focus({ preventScroll: true });
    restoreFileFocus = null;
  }
  status.textContent = message;
};
systemTheme.addEventListener("change", ({ matches }) => {
  if (state?.theme === "system") applyEditorAppearance();
  window.ipc.postMessage(JSON.stringify({ command: "system_appearance", dark: matches }));
});
window.ipc.postMessage(JSON.stringify({ command: "ready", system_dark: systemTheme.matches }));
