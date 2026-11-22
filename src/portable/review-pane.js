"use strict";

let reviewPath = null;
let focusedReview = null;
let backendFocus = null;
let reviewDelete = null;
let openOnly = false;

function reviewSelection() {
  if (isSource()) {
    const start = source.selectionStart;
    const end = source.selectionEnd;
    return start === end ? null : { kind: "source", start, end };
  }
  const parts = visualParts();
  return parts.length ? { kind: "visual", first: parts[0], last: parts.at(-1) } : null;
}

function selectedReview() {
  return state.document?.reviews.find((record) => record.id === focusedReview);
}

function reviewCommand(op, fields = {}) {
  if (!state.document?.showReviews) return;
  showReviews = true;
  send("review", { op, ...fields });
}

function addReview() {
  const selection = reviewSelection();
  if (!selection) {
    status.textContent = state.labels.selectForReview;
    return;
  }
  reviewCommand("start", { selection });
}

function preserveSelection(control) {
  control.addEventListener("pointerdown", (event) => {
    if (event.button === 0) event.preventDefault();
  });
  return control;
}

function paintReviewFocus() {
  const index = state.document?.reviews.findIndex((record) => record.id === focusedReview) ?? -1;
  for (const mark of preview.querySelectorAll("mark[data-reviews]")) {
    mark.classList.toggle("review-focused", mark.dataset.reviews.split(" ").includes(String(index)));
  }
}

function focusReview(record) {
  focusedReview = record.id;
  reviewDelete = null;
  renderReviews();
  if (!record.location) return;
  if (isSource()) {
    jump(record.location, true, true);
  } else {
    const mark = preview.querySelector(".review-focused");
    if (mark) mark.scrollIntoView({ block: "center" });
    else jump(record.location);
  }
}

function reviewMarkedClick(event) {
  const marker = event.target.closest("mark[data-reviews]");
  if (!marker || !getSelection().isCollapsed) return;
  const index = Number(marker.dataset.reviews.split(" ")[0]);
  const record = state.document?.reviews[index];
  if (record) {
    showReviews = true;
    focusedReview = record.id;
    reviewDelete = null;
    renderReviews();
    document.querySelector(".review-card.selected")?.scrollIntoView({ block: "nearest" });
  }
}

function renderReviewDraft(draft) {
  const t = state.labels;
  const form = element("form", undefined, "review-card review-draft");
  form.dataset.reviewDraft = "";
  form.append(element("strong", draft.quote ? `«${draft.quote}»` : t.newReview, "review-quote"));
  const input = element("textarea", undefined, "review-input");
  input.id = "review-draft-input";
  input.setAttribute("aria-label", t.comment);
  input.value = draft.text;
  input.rows = 4;
  const actions = element("div", undefined, "review-card-actions");
  const apply = element("button", t.saveComment, "review-apply");
  apply.type = "submit";
  form.addEventListener("submit", (event) => {
    event.preventDefault();
    reviewCommand("commit", { text: input.value });
  });
  actions.append(button(t.cancel, () => reviewCommand("cancel")), apply);
  form.append(input, actions);
  return form;
}

function requestReviewDelete(record) {
  if (!record) return;
  reviewDelete = record.id;
  renderReviews();
  byId("review-delete-cancel")?.focus();
}

function reattachReview(record) {
  const selection = reviewSelection();
  if (selection) reviewCommand("reattach", { id: record.id, selection });
  else status.textContent = state.labels.selectForReview;
}

function renderReviewCard(record, drafting) {
  const t = state.labels;
  const focused = record.id === focusedReview;
  const card = element("div", undefined, "review-card");
  card.dataset.reviewId = record.id;
  card.classList.toggle("selected", focused);
  const body = button(undefined, () => focusReview(record), "review-card-body");
  body.disabled = drafting;
  body.setAttribute("aria-pressed", focused);
  const binding = record.location ? `«${record.quote}»` : record.ambiguous ? t.ambiguous : t.unattached;
  body.append(element("strong", binding, "review-quote"), element("span", record.text));
  const date = record.date == null ? null : new Date(record.date);
  const when = date && Number.isFinite(date.getTime())
    ? new Intl.DateTimeFormat(state.english ? "en" : "ru", { dateStyle: "short" }).format(date)
    : "";
  const metadata = [record.status === "resolved" ? t.resolved : "", when].filter(Boolean);
  if (metadata.length) body.append(element("small", metadata.join(" · "), "review-state"));
  card.append(body);
  if (drafting) return card;

  if (reviewDelete === record.id) {
    const actions = element("div", undefined, "review-card-actions");
    card.append(element("p", t.confirmDelete, "review-confirm"));
    const cancel = button(t.cancel, () => {
      reviewDelete = null;
      renderReviews();
    });
    cancel.id = "review-delete-cancel";
    actions.append(button(t.deleteReview, () => {
      reviewDelete = null;
      reviewCommand("delete", { id: record.id });
    }), cancel);
    card.append(actions);
  } else {
    const options = element("details", undefined, "review-more");
    const summary = labelControl(preserveSelection(element("summary")), t.more);
    summary.append(icon("more"));
    const extra = element("div", undefined, "review-more-actions menu-popup");
    extra.append(
      button(t.editReview, () => reviewCommand("edit", { id: record.id })),
      button(record.status === "resolved" ? t.reopenReview : t.resolveReview,
        () => reviewCommand("toggle", { id: record.id })),
      preserveSelection(button(t.reattachReview, () => {
      options.open = false;
      reattachReview(record);
    })), button(t.deleteReview, () => requestReviewDelete(record)));
    options.append(summary, extra);
    options.addEventListener("toggle", () => {
      if (!options.open) return;
      const anchor = summary.getBoundingClientRect();
      extra.style.right = (window.innerWidth - anchor.right) + "px";
      extra.style.top = (anchor.bottom + extra.offsetHeight <= window.innerHeight
        ? anchor.bottom : Math.max(0, anchor.top - extra.offsetHeight)) + "px";
    });
    card.append(options);
  }
  return card;
}

function renderReviews() {
  const t = state.labels;
  const doc = state.document;
  const list = byId("reviews-content");
  const toolbar = byId("review-actions");
  const available = doc?.showReviews;
  const editor = doc?.reviewEditor;
  const draft = editor?.draft;
  const previousScroll = list.scrollTop;
  const path = state.tabs[state.active]?.path ?? null;
  if (path !== reviewPath) {
    reviewPath = path;
    focusedReview = null;
    backendFocus = null;
    reviewDelete = null;
  }
  if (editor?.focus !== backendFocus) {
    backendFocus = editor?.focus;
    focusedReview = backendFocus;
  }
  if (focusedReview && !doc?.reviews.some((record) => record.id === focusedReview)) focusedReview = null;
  byId("reviews").hidden = !available || !showReviews;
  byId("reviews-edge").hidden = !available || showReviews;
  paneToggle("open-reviews", t.showComments, false, "left", "comments");
  paneToggle("hide-reviews", t.hideComments, true, "right");
  byId("hide-reviews").disabled = !!draft;
  byId("reviews-title").textContent = t.reviews;
  list.replaceChildren();
  toolbar.replaceChildren();
  paintSource();
  if (!available) return;
  const add = labelControl(preserveSelection(button(undefined, addReview)), t.addReview);
  add.append(icon("plus"));
  add.disabled = !!draft;
  toolbar.append(add);
  if (doc.reviews.some((record) => record.status === "resolved")) {
    const filter = button(openOnly ? t.openComments : t.all, () => {
      openOnly = !openOnly;
      renderReviews();
    });
    filter.disabled = !!draft;
    filter.setAttribute("aria-pressed", openOnly);
    list.append(element("div", undefined, "review-filter"));
    list.lastChild.append(filter);
  }
  if (draft && !draft.id) list.append(renderReviewDraft(draft));
  for (const record of doc.reviews) {
    if (draft?.id === record.id) list.append(renderReviewDraft(draft));
    else if (!openOnly || record.status !== "resolved") list.append(renderReviewCard(record, !!draft));
  }
  if (!doc.reviews.length && !draft) list.append(element("div", t.emptyReview, "panel-note"));
  list.scrollTop = previousScroll;
  paintReviewFocus();
}

document.getElementById("reviews-content").addEventListener("scroll", (event) => {
  for (const options of event.currentTarget.querySelectorAll(".review-more[open]")) options.open = false;
});

function reviewKey(event, key) {
  const draft = state.document?.reviewEditor?.draft;
  const primary = event.ctrlKey || event.metaKey;
  if (draft) {
    if (event.key === "Escape") return () => reviewCommand("cancel");
    if (primary && event.key === "Enter") {
      return () => reviewCommand("commit", { text: byId("review-draft-input").value });
    }
    return null;
  }
  if (!state.document?.showReviews || !(primary && event.altKey)) return null;
  if (key === "m" || event.code === "KeyM") return addReview;
  const selected = selectedReview();
  if ((key === "e" || event.code === "KeyE") && selected) return () => reviewCommand("edit", { id: selected.id });
  if ((key === "r" || event.code === "KeyR") && selected) return () => reviewCommand("toggle", { id: selected.id });
  if ((key === "l" || event.code === "KeyL") && selected) return () => reattachReview(selected);
  if ((key === "d" || event.code === "KeyD") && selected) return () => requestReviewDelete(selected);
  return null;
}
