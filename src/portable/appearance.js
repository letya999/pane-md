"use strict";

let editorFonts;
let refreshFontChoices;

function activeColorTheme() {
  return state.theme === "dark" || state.theme === "system" && systemTheme.matches ? "dark" : "light";
}

function hexColor(value) {
  return "#" + value.toString(16).padStart(6, "0");
}

function applyEditorAppearance() {
  const appearance = state.appearance;
  const { rich, source } = appearance.settings;
  const palette = appearance[activeColorTheme()];
  const style = byId("document-area").style;
  const family = (font, fallback) => font.family ? JSON.stringify(font.family) + ", " + fallback : fallback;
  style.setProperty("--doc-rich-font", family(rich, "system-ui, sans-serif"));
  style.setProperty("--doc-source-font", family(source, 'ui-monospace, "DejaVu Sans Mono", monospace'));
  style.setProperty("--doc-rich-size", rich.half_points / 2 + "pt");
  style.setProperty("--doc-source-size", source.half_points / 2 + "pt");
  updateSourceLineHeight(source.half_points, zoom);
  for (const key of ["canvas", "text", "muted", "line", "accent", "highlight", "marked", "header", "stripe"]) {
    style.setProperty("--doc-" + key, hexColor(palette[key]));
  }
}

function showEditorAppearance() {
  const t = state.labels;
  const dialog = byId("appearance");
  const current = state.appearance;
  let draft = JSON.parse(JSON.stringify(current.settings));
  let selectedTheme = activeColorTheme();
  const form = element("form");
  const title = element("h2", t.appearance);
  title.id = "appearance-title";
  form.append(title);

  const labeled = (name, input, className) => {
    const label = element("label", undefined, className);
    label.append(element("span", name.replace(/…$/, "")), input);
    return label;
  };
  const fontFields = [];
  for (const [key, label] of [["rich", t.richFont], ["source", t.sourceFont]]) {
    const row = element("div", undefined, "appearance-font-row");
    const families = element("select");
    const size = element("input");
    size.type = "number";
    size.min = "8";
    size.max = "36";
    size.step = "0.5";
    size.required = true;
    fontFields.push({ key, families, size });
    row.append(labeled(label, families), labeled(t.fontSize, size));
    form.append(row);
  }

  refreshFontChoices = (reset) => {
    for (const { key, families, size } of fontFields) {
      const selected = reset ? draft[key].family : families.value;
      families.replaceChildren(new Option(t.defaultFont, ""));
      const names = [...new Set([...(editorFonts ?? []), ...(selected ? [selected] : [])])];
      names.sort((a, b) => a.localeCompare(b));
      for (const name of names) families.add(new Option(name, name));
      families.value = selected;
      if (reset) size.value = draft[key].half_points / 2;
    }
  };
  refreshFontChoices(true);

  const themes = element("select");
  themes.add(new Option(t.light, "light"));
  themes.add(new Option(t.dark, "dark"));
  themes.value = selectedTheme;
  const colors = element("div", undefined, "appearance-colors");
  const colorInputs = [];
  for (const [key, paletteKey, label] of [
    ["paper", "canvas", t.paperColor], ["ink", "text", t.textColor], ["link", "accent", t.linkColor],
  ]) {
    const group = element("span", undefined, "appearance-color-field");
    const text = element("input");
    text.type = "text";
    text.maxLength = 7;
    text.pattern = "#[0-9a-fA-F]{6}";
    text.required = true;
    text.spellcheck = false;
    const swatch = element("input");
    swatch.type = "color";
    swatch.setAttribute("aria-label", label.replace(/…$/, ""));
    text.addEventListener("input", () => {
      if (!text.validity.valid) return;
      swatch.value = text.value;
      draft[selectedTheme][key] = parseInt(text.value.slice(1), 16);
    });
    swatch.addEventListener("input", () => {
      text.value = swatch.value;
      draft[selectedTheme][key] = parseInt(swatch.value.slice(1), 16);
    });
    colorInputs.push({ key, paletteKey, text, swatch });
    group.append(swatch, labeled(label, text));
    colors.append(group);
  }

  const fillColors = () => {
    const overrides = draft[selectedTheme];
    const saved = current.settings[selectedTheme];
    const canUseEffective = overrides.paper === saved.paper;
    for (const { key, paletteKey, swatch, text } of colorInputs) {
      const fallback = (canUseEffective ? current[selectedTheme] : current.defaults[selectedTheme])[paletteKey];
      text.value = swatch.value = hexColor(overrides[key] ?? fallback);
    }
  };
  themes.addEventListener("change", () => {
    const invalid = colorInputs.find(({ text }) => !text.validity.valid);
    if (invalid) {
      themes.value = selectedTheme;
      invalid.text.reportValidity();
      return;
    }
    selectedTheme = themes.value;
    fillColors();
  });
  fillColors();

  const resetColors = button(t.resetColors, () => {
    draft[selectedTheme] = { ...current.defaults.settings[selectedTheme] };
    fillColors();
  });
  const error = element("p", "", "appearance-error");
  error.id = "appearance-error";
  error.setAttribute("role", "alert");
  error.hidden = true;
  const sharedActions = element("div", undefined, "appearance-actions");
  const reset = button(t.resetAppearance, () => {
    draft = JSON.parse(JSON.stringify(current.defaults.settings));
    refreshFontChoices(true);
    fillColors();
  });
  const cancel = button(t.cancel, () => { if (!apply.disabled) dialog.close(); });
  const apply = element("button", t.apply, "appearance-apply");
  apply.type = "submit";
  apply.id = "appearance-apply";
  sharedActions.append(reset, cancel, apply);
  form.append(labeled(t.themeColors, themes, "appearance-theme"), colors, resetColors,
    element("p", t.contrastHint, "appearance-hint"), error, sharedActions);
  form.addEventListener("submit", (event) => {
    event.preventDefault();
    for (const { key, families, size } of fontFields) {
      draft[key] = { family: families.value, half_points: size.valueAsNumber * 2 };
    }
    error.hidden = true;
    apply.disabled = true;
    window.ipc.postMessage(JSON.stringify({ command: "set_editor_appearance", appearance: draft }));
  });
  dialog.replaceChildren(form);
  dialog.showModal();
  if (editorFonts === undefined) {
    editorFonts = [];
    window.ipc.postMessage(JSON.stringify({ command: "appearance_fonts" }));
  }
}

document.getElementById("appearance").addEventListener("cancel", (event) => {
  if (byId("appearance-apply").disabled) event.preventDefault();
});
document.getElementById("appearance").addEventListener("close", () => {
  refreshFontChoices = null;
  document.querySelector("#menu details:last-child > summary")?.focus();
});

window.setEditorFonts = (fonts) => {
  editorFonts = fonts;
  refreshFontChoices?.(false);
};

window.setEditorAppearance = (appearance) => {
  state.appearance = appearance;
  applyEditorAppearance();
  byId("appearance").close();
};

window.editorAppearanceError = (message) => {
  const error = byId("appearance-error");
  error.textContent = message;
  error.hidden = false;
  byId("appearance-apply").disabled = false;
};
