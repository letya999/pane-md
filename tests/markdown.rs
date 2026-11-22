use pane_md::{
    edit_source,
    markdown::{Layout, Markdown, TableEdit, table_patch},
    review::{self, Binding, Review, Status},
    source_view, utf16_byte,
};

fn range_for(text: &str, quote: &str) -> std::ops::Range<usize> {
    let start = text.find(quote).expect("visible quote");
    text[..start].encode_utf16().count()..text[..start + quote.len()].encode_utf16().count()
}

#[test]
fn source_edit_preserves_mixed_endings_and_unicode_boundaries() {
    let mut source = "Первый\r\n😀\nпоследний\r".to_owned();
    assert_eq!(utf16_byte("Ж😀", 1), Some(2));
    assert_eq!(utf16_byte("Ж😀", 2), None);
    assert_eq!(utf16_byte("Ж😀", 3), Some(6));
    let (before, map) = source_view(&source);
    let after = "Первый\r😃\rпоследний\r".encode_utf16().collect::<Vec<_>>();
    let (range, text) = edit_source(&source, &before, &map, &after).unwrap();
    assert_eq!(&source[range.clone()], "😀");
    source.replace_range(range, &text);
    assert_eq!(source, "Первый\r\n😃\nпоследний\r");
}

#[test]
fn localized_edits_keep_author_formatting() {
    let mut source = "---\r\nowner: me\r\n---\r\n\r\n# Lead time\r\n\r\nKeep **P85** and _spacing_.\r\n\r\n| A | B |\r\n| :- | -: |\r\n| Median / P85 by team | Ж😀 and a\\|b |\r\n\r\n+  unchanged\r\n\r\n<!-- author's comment -->\r\n".to_owned();
    let original = source.clone();
    let doc = Markdown::parse(&source);
    let para = doc
        .regions
        .iter()
        .find(|r| r.text.starts_with("Keep"))
        .unwrap();
    let (range, text) = para
        .patch(&source, range_for(&para.text, "P85"), "P90")
        .unwrap();
    assert_eq!(&source[range.clone()], "P85");
    source.replace_range(range, &text);
    let doc = Markdown::parse(&source);
    let cell = doc
        .regions
        .iter()
        .find(|r| r.text == "Ж😀 and a|b")
        .unwrap();
    let raw = cell
        .source_selection(range_for(&cell.text, "😀 and a|"))
        .unwrap();
    assert_eq!(&source[raw], "😀 and a\\|");
    let (range, text) = cell
        .patch(&source, range_for(&cell.text, "Ж😀"), "Ю🙂")
        .unwrap();
    source.replace_range(range, &text);
    assert_eq!(
        source,
        original
            .replacen("**P85**", "**P90**", 1)
            .replacen("Ж😀", "Ю🙂", 1)
    );
}

#[test]
fn review_round_trip_and_recovery() {
    let before = "# Lead time\n\n| One | Two |\n| --- | --- |\n| Ж😀 | Median / **P85** by team |\n\n<!-- regular -->\n\nTail\n";
    let doc = Markdown::parse(before);
    let (id, cell) = doc
        .regions
        .iter()
        .enumerate()
        .find(|(_, r)| r.text.contains("P85"))
        .unwrap();
    let (after, quote, anchor) = review::capture(&doc, id, range_for(&cell.text, "P85")).unwrap();
    assert_eq!((anchor.row, anchor.cell), (Some(1), Some(1)));
    let record = Review {
        v: 1,
        id: "a".into(),
        quote,
        anchor,
        text: "Кириллица 😀 -->\nещё".into(),
        status: Status::Open,
        updated_at: None,
    };
    let encoded = record.encode();
    assert_eq!(encoded.matches("-->").count(), 1);
    assert!(encoded.contains("\\u002D\\u002D>"));
    assert!(!encoded.contains('\n'));
    let mut source = before.to_owned();
    source.insert_str(after, &review::insertion(before, after, &record));
    assert_eq!(&source[..after], &before[..after]);
    assert!(source[after..].starts_with("\n\n<!-- pane-md:review "));
    let doc = Markdown::parse(&source);
    assert!(doc.diagnostics.is_empty());
    assert_eq!(doc.records[0].review, record);
    let legacy = source.replace("<!-- pane-md:review", "<!-- notepad-md:review");
    assert_eq!(Markdown::parse(&legacy).records[0].review, record);
    let Binding::Attached {
        line,
        column,
        bytes,
        ..
    } = &doc.records[0].binding
    else {
        panic!("should attach");
    };
    assert_eq!(
        (*line, *column),
        (5, before.lines().nth(4).unwrap().find("P85").unwrap() + 1)
    );
    assert_eq!(&source[bytes.clone()], "P85");
    let shifted = source.replacen("# Lead time\n\n", "# Lead time\n\nNew paragraph.\n\n", 1);
    assert!(matches!(
        Markdown::parse(&shifted).records[0].binding,
        Binding::Attached { line: 7, .. }
    ));
    let ambiguous = source.replacen(
        "| Ж😀 | Median / **P85** by team |",
        "| Ж😀 | Median / **P85** by team |\n| Ж😀 | Median / **P85** by team |",
        1,
    );
    assert_eq!(
        Markdown::parse(&ambiguous).records[0].binding,
        Binding::Ambiguous
    );
    let missing = source.replacen("**P85**", "**P90**", 1);
    assert_eq!(
        Markdown::parse(&missing).records[0].binding,
        Binding::Missing
    );
    let code = format!(
        "```md\n{encoded}\n```\n\n{source}\n<!-- pane-md:review {{broken}} -->\n\n<!-- pane-md:review {{\"v\":99}} -->\n"
    );
    let doc = Markdown::parse(&code);
    assert_eq!(doc.records.len(), 1);
    assert_eq!(doc.diagnostics.len(), 1);
    assert!(code.contains("<!-- regular -->"));
}

#[test]
fn exact_context_distinguishes_repeated_quotes_and_keeps_html_safe() {
    let before =
        "# Глава\n\nЖ😀 &amp; P85; left P85; right.\n\n- item **Ж😀**\n- second\n\nAfter\n";
    let doc = Markdown::parse(before);
    let (i, paragraph) = doc
        .regions
        .iter()
        .enumerate()
        .find(|(_, r)| r.text.starts_with("Ж😀"))
        .unwrap();
    let (after, quote, anchor) =
        review::capture(&doc, i, range_for(&paragraph.text, "Ж😀 & P85")).unwrap();
    assert_eq!(quote, "Ж😀 & P85");
    assert_eq!(
        &before[paragraph
            .source_selection(range_for(&paragraph.text, &quote))
            .unwrap()],
        "Ж😀 &amp; P85"
    );
    let review = Review {
        v: 1,
        id: "unicode".into(),
        quote,
        anchor,
        text: "ok".into(),
        status: Status::Resolved,
        updated_at: None,
    };
    let mut source = before.to_owned();
    source.insert_str(after, &review::insertion(before, after, &review));
    assert!(matches!(
        Markdown::parse(&source).records[0].binding,
        Binding::Attached { .. }
    ));
    let (i, list) = doc
        .regions
        .iter()
        .enumerate()
        .find(|(_, r)| r.text.contains("item"))
        .unwrap();
    let (after, _, _) = review::capture(&doc, i, range_for(&list.text, "Ж😀")).unwrap();
    assert_eq!(
        &before[..after],
        "# Глава\n\nЖ😀 &amp; P85; left P85; right.\n\n- item **Ж😀**\n- second"
    );
}

#[test]
fn review_spanning_paragraphs_rebinds_without_editing_markdown() {
    let part = "First **Ж😀** &amp; end\n\nThen P85 last";
    let original = format!("# Section\r\n\r\n{part}\r\n\r\nUnchanged");
    let doc = Markdown::parse(&original);
    let first = &doc.regions[1];
    let last = &doc.regions[2];
    let (insert, quote, anchor) = review::capture_span(
        &doc,
        (1, range_for(&first.text, "Ж😀 & end")),
        (2, range_for(&last.text, "Then P85")),
    )
    .unwrap();
    assert_eq!(&original[..insert], format!("# Section\r\n\r\n{part}"));
    assert_eq!(quote, "Ж😀 & end\nThen P85");
    let record = Review {
        v: 1,
        id: "span".into(),
        quote,
        anchor,
        text: "Long note -->".repeat(400),
        status: Status::Open,
        updated_at: Some(1_765_000_123_456),
    };
    let addition = review::insertion(&original, insert, &record);
    let mut source = original.clone();
    source.insert_str(insert, &addition);
    let parsed = Markdown::parse(&source);
    assert_eq!(parsed.records[0].review, record);
    let Binding::Attached {
        ref bytes,
        line: 3,
        tail: Some(_),
        ..
    } = parsed.records[0].binding
    else {
        panic!("the full multi-block selection should bind");
    };
    assert_eq!(&source[bytes.clone()], "Ж😀** &amp; end\n\nThen P85");
    let moved = source.replacen(part, &format!("Inserted\n\n{part}"), 1);
    assert!(matches!(
        Markdown::parse(&moved).records[0].binding,
        Binding::Attached { line: 5, .. }
    ));
    let duplicated = source.replacen(part, &format!("{part}\n\n{part}"), 1);
    assert_eq!(
        Markdown::parse(&duplicated).records[0].binding,
        Binding::Ambiguous
    );
    let removed = source.replacen("Then P85", "Then P90", 1);
    assert_eq!(
        Markdown::parse(&removed).records[0].binding,
        Binding::Missing
    );
}

#[test]
fn table_structure_changes_only_the_table() {
    let source = "before\r\n\r\n| A | B |\r\n| :- | -: |\r\n| x | y |\r\n\r\nafter";
    let doc = Markdown::parse(source);
    let (range, rows) = doc
        .layout
        .iter()
        .find_map(|layout| match layout {
            Layout::Table { source, rows, .. } => Some((source.clone(), rows.len())),
            _ => None,
        })
        .unwrap();
    for operation in [
        TableEdit::AddColumn,
        TableEdit::DeleteColumn,
        TableEdit::AddRow,
        TableEdit::DeleteRow,
    ] {
        let mut changed = source.to_owned();
        let patch = table_patch(source, range.clone(), rows, 2, 1, 1, operation).unwrap();
        changed.replace_range(range.clone(), &patch);
        assert!(changed.starts_with("before\r\n\r\n"));
        assert!(changed.ends_with("\r\n\r\nafter"));
        assert!(
            Markdown::parse(&changed)
                .layout
                .iter()
                .any(|layout| matches!(layout, Layout::Table { .. }))
        );
    }
}

#[test]
fn table_navigation_includes_implicit_cells_and_stays_in_one_table() {
    let source =
        "> | A | B | C |\n> | --- | --- | --- |\n> | short |\n>\n> | D | E |\n> | --- | --- |";
    let doc = Markdown::parse(source);
    let Layout::Table { rows, .. } = &doc.layout[0] else {
        panic!("expected the first table");
    };
    let cells: Vec<_> = rows.iter().flatten().copied().collect();
    for pair in cells.windows(2) {
        assert_eq!(doc.table_neighbor(pair[0], false), Some(pair[1]));
        assert_eq!(doc.table_neighbor(pair[1], true), Some(pair[0]));
    }
    assert_eq!(doc.table_neighbor(cells[0], true), None);
    assert_eq!(doc.table_neighbor(*cells.last().unwrap(), false), None);

    for (edit, cell, expected) in [
        (TableEdit::AddRow, rows[1][2], (2, 0)),
        (TableEdit::AddColumn, rows[1][1], (1, 2)),
        (TableEdit::DeleteRow, rows[1][1], (0, 1)),
        (TableEdit::DeleteColumn, rows[1][2], (1, 1)),
    ] {
        let patch = doc
            .edit_table(source, cell, edit)
            .unwrap_or_else(|error| panic!("table edit focusing {expected:?}: {error:?}"));
        let mut text = source.to_owned();
        text.replace_range(patch.range, &patch.text);
        let current = Markdown::parse(&text);
        let focused = &current.regions[patch.focus_region.unwrap()];
        assert_eq!(focused.row.zip(focused.cell), Some(expected));
        assert_eq!(patch.caret.start, focused.source.start);
    }
}

#[test]
fn editing_an_implicit_cell_materializes_only_the_needed_columns() {
    let source = "Intro\r\n\r\n| A | B | C | D |\r\n| :--- | --- | ---: | :---: |\r\n| x |\r\nOne | Two  \r\n\r\nAfter";
    let doc = Markdown::parse(source);
    let Layout::Table { rows, .. } = &doc.layout[1] else {
        panic!("expected a table");
    };
    let (range, text) = doc.regions[rows[1][3]].patch(source, 0..0, "Ж|😀").unwrap();
    let mut edited = source.to_owned();
    edited.replace_range(range, &text);
    assert_eq!(edited, source.replace("| x |", "| x |   |   | Ж\\|😀 |"));
    let next = Markdown::parse(&edited);
    assert_eq!(next.regions[rows[1][3]].text, "Ж|😀");
    assert!(next.regions[rows[1][1]].text.is_empty());

    let edit = doc
        .paste_text(source, &[(rows[2][3], 0..0)], " new\nline ")
        .unwrap();
    let mut edited = source.to_owned();
    edited.replace_range(edit.range, &edit.text);
    assert_eq!(
        edited,
        source.replace("One | Two", "One | Two |   | &#32;new<br>line&#32;")
    );
    assert!(edited[..edit.caret.start].ends_with("line&#32;"));
    let next = Markdown::parse(&edited);
    assert_eq!(next.regions[rows[2][3]].text, " new\u{000b}line ");
}

#[test]
fn table_commands_keep_sparse_rows_and_extra_unrendered_cells() {
    let source = "| A | B | C |\r\n| :--- | ---: | :---: |\r\n| Short |\r\nOne | Two  \r\n| With | Value | Tail | Hidden |\r\n\r\nAfter";
    let doc = Markdown::parse(source);
    let Layout::Table { rows, .. } = &doc.layout[0] else {
        panic!("expected a table");
    };
    let patch = |row: usize, col: usize, edit| {
        let edit = doc.edit_table(source, rows[row][col], edit).unwrap();
        let mut edited = source.to_owned();
        edited.replace_range(edit.range, &edit.text);
        edited
    };
    let added = patch(1, 1, TableEdit::AddColumn);
    assert_eq!(
        added,
        "| A | B |   | C |\r\n| :--- | ---: | --- | :---: |\r\n| Short |\r\nOne | Two|   |  \r\n| With | Value |   | Tail | Hidden |\r\n\r\nAfter"
    );
    let parsed = Markdown::parse(&added);
    let Layout::Table {
        rows, alignment, ..
    } = &parsed.layout[0]
    else {
        panic!("adding a column changed table structure");
    };
    assert_eq!(alignment.len(), 4);
    assert_eq!(parsed.regions[rows[3][3]].text, "Tail");
    assert!(!parsed.regions.iter().any(|region| region.text == "Hidden"));

    let deleted = patch(1, 0, TableEdit::DeleteColumn);
    assert_eq!(
        deleted,
        "| B | C |\r\n| ---: | :---: |\r\n|   |\r\n Two  \r\n| Value | Tail | Hidden |\r\n\r\nAfter"
    );
    let parsed = Markdown::parse(&deleted);
    let Layout::Table { rows, .. } = &parsed.layout[0] else {
        panic!("deleting a column changed table structure");
    };
    assert_eq!(rows.len(), 4);
    assert!(parsed.regions[rows[1][0]].text.is_empty());

    let promoted = patch(0, 0, TableEdit::DeleteRow);
    assert!(promoted.starts_with("| Short |   |   |\r\n| :--- | ---: | :---: |"));
    assert!(matches!(
        Markdown::parse(&promoted).layout[0],
        Layout::Table { .. }
    ));
}

#[test]
fn editing_nested_sparse_tables_preserves_quote_and_list_prefixes() {
    for (source, cell_line, expected) in [
        (
            "> > | A | B |\r\n> > | --- | :---: |\r\n> > | x |\r\n>\r\n> After",
            "> > | x |",
            "> > | x | filled |",
        ),
        (
            "- | A | B |\n  | --- | :---: |\n  | x |\n\nAfter",
            "  | x |",
            "  | x | filled |",
        ),
    ] {
        let doc = Markdown::parse(source);
        let Layout::Table { rows, .. } = &doc.layout[0] else {
            panic!("expected nested table in {source:?}");
        };
        let patch = doc
            .paste_text(source, &[(rows[1][1], 0..0)], "filled")
            .unwrap();
        let mut text = source.to_owned();
        text.replace_range(patch.range, &patch.text);
        assert_eq!(text, source.replace(cell_line, expected));

        let patch = doc
            .edit_table(source, rows[0][0], TableEdit::AddRow)
            .unwrap();
        let mut text = source.to_owned();
        text.replace_range(patch.range, &patch.text);
        let prefix = cell_line.split('|').next().unwrap();
        let separator = if source.contains('\r') { "\r\n" } else { "\n" };
        assert_eq!(
            text,
            source.replace(
                cell_line,
                &format!("{prefix}|   |   |{separator}{cell_line}")
            )
        );
        let patch = doc
            .edit_table(source, rows[1][1], TableEdit::AddColumn)
            .unwrap();
        let mut text = source.to_owned();
        text.replace_range(patch.range, &patch.text);
        assert!(text.contains(&format!("{prefix}| --- | :---: | --- |")));
        assert!(text.contains(cell_line));
    }
}

#[test]
fn lint_uses_the_same_heading_slugs_and_reports_broken_reviews() {
    let source = "\
# Заголовок

# Заголовок

[One](#заголовок) [Two](#заголовок-1) [Missing](#other)
[Bad encoding](#%00).

A paragraph.

<!-- pane-md:review {broken} -->

<!-- pane-md:review {\"v\":1,\"id\":\"r-1\",\"quote\":\"nowhere\",\"anchor\":{\"block\":\"paragraph\",\"prefix\":\"\",\"suffix\":\"\"},\"text\":\"fix\",\"status\":\"open\"} -->
";
    let issues = pane_md::lint::check(source);
    assert_eq!(
        issues
            .iter()
            .map(|issue| (issue.code, issue.line))
            .collect::<Vec<_>>(),
        [
            ("anchor-missing", 5),
            ("anchor-encoding", 6),
            ("review-record", 10),
            ("review-anchor-missing", 12),
        ]
    );
}

#[test]
fn file_lint_checks_local_targets_with_the_same_paths_and_anchors_as_navigation() {
    use std::fs;

    let project = tempfile::tempdir().unwrap();
    let docs = project.path().join("docs");
    fs::create_dir(&docs).unwrap();
    fs::write(
        docs.join("Глава #1.MARKDOWN"),
        "\u{feff}# План\r\n\r\n# План\r\n",
    )
    .unwrap();
    fs::write(docs.join("invalid.md"), b"\xff").unwrap();
    let path = project.path().join("README.md");
    fs::write(&path, "# Сохранённая версия\n").unwrap();
    let source = "\
# Текущий

[Self](./README.md#текущий)
[Other][target]
[Missing heading](docs/Глава%20%231.MARKDOWN#отсутствует)
[Missing attachment](docs/note.pdf)
[![Missing image](docs/absent.png)](docs/Глава%20%231.MARKDOWN#план)
![Embedded](data:image/png;base64,AA)
[Folder](docs/)
[External](https://example.invalid/missing)
[Bad encoding](docs/%ZZ.md)
[Unreadable Markdown](docs/invalid.md#heading)

[target]: docs/Глава%20%231.MARKDOWN#план-1
";
    assert!(pane_md::lint::check(source).is_empty());
    let issues = pane_md::lint::check_at(source, &path);
    assert_eq!(
        issues
            .iter()
            .map(|issue| (issue.code, issue.line, issue.column))
            .collect::<Vec<_>>(),
        [
            ("anchor-missing", 5, 1),
            ("local-target-missing", 6, 1),
            ("local-target-missing", 7, 2),
            ("local-target-encoding", 11, 1),
            ("local-target-unreadable", 12, 1),
        ]
    );
    assert_eq!(fs::read_to_string(&path).unwrap(), "# Сохранённая версия\n");
}

#[test]
fn source_links_cover_labels_destinations_and_reference_uses() {
    let source = "# Начало\r\n\r\n`[skip](#начало)` [**Пуск**](#начало) [logo][site] [![icon](logo.svg)](guide.md)\r\n\r\n[site]: https://example.org\r\n";
    let doc = Markdown::parse(source);
    let link = |needle: &str| doc.source_link(source.find(needle).unwrap());
    assert!(link("skip").is_none());
    assert_eq!(link("Пуск").unwrap().target, "#начало");
    assert_eq!(link("#начало) [logo]").unwrap().target, "#начало");
    assert_eq!(link("logo][site]").unwrap().target, "https://example.org");
    assert_eq!(link("icon").unwrap().target, "guide.md");
    assert!(link("https://example.org").is_none());
}

#[test]
fn visual_link_edits_preserve_markup_titles_and_other_occurrences() {
    let source =
        "Before [**Ж😀** and *later*](<old path.md>   \"Keep &copy;\") [Another](old%20path.md).";
    let doc = Markdown::parse(source);
    let selected = range_for(&doc.regions[0].text, "later");
    assert_eq!(
        doc.link_edit(0, selected.clone()).unwrap().target(),
        Some("old path.md")
    );
    assert_eq!(
        doc.link_edit(0, selected.end..selected.end)
            .unwrap()
            .target(),
        Some("old path.md")
    );
    let patch = doc
        .change_link(source, 0, selected.clone(), "new (file).md#начало")
        .unwrap();
    assert_eq!(&source[patch.range.clone()], "old path.md");
    let mut edited = source.to_owned();
    edited.replace_range(patch.range, &patch.text);
    assert_eq!(
        edited,
        source.replace("old path.md", "new%20%28file%29.md#начало")
    );
    let changed = Markdown::parse(&edited);
    let patch = changed.change_link(&edited, 0, selected, "").unwrap();
    edited.replace_range(patch.range, &patch.text);
    assert_eq!(
        edited,
        "Before **Ж😀** and *later* [Another](old%20path.md)."
    );
}

#[test]
fn visual_reference_link_edits_leave_the_shared_definition_intact() {
    let source =
        "[**First**][ref], [ref][], and [ref].\r\n\r\n[ref]: <old path.md> \"Shared tip\"\r\n";
    let doc = Markdown::parse(source);
    let text = &doc.regions[0].text;
    let position = range_for(text, "First").start;
    let patch = doc
        .change_link(source, 0, position..position, "#updated")
        .unwrap();
    let mut edited = source.to_owned();
    edited.replace_range(patch.range, &patch.text);
    assert_eq!(
        edited,
        source.replace("[**First**][ref]", "[**First**](#updated \"Shared tip\")")
    );

    let position = range_for(text, "ref.").start;
    let patch = doc.change_link(source, 0, position..position, "").unwrap();
    let mut removed = source.to_owned();
    removed.replace_range(patch.range, &patch.text);
    assert_eq!(removed, source.replace("and [ref].", "and ref."));
    assert_eq!(Markdown::parse(&removed).source_links.len(), 2);
}

#[test]
fn visual_autolinks_can_change_destination_or_become_plain_text() {
    for (source, visible) in [
        ("<https://example.org/guide>", "https://example.org/guide"),
        ("https://example.org/guide", "https://example.org/guide"),
        ("www.example.org", "www.example.org"),
        ("person@example.org", "person@example.org"),
        ("[https://example.org](old.md)", "https://example.org"),
    ] {
        let doc = Markdown::parse(source);
        assert_eq!(doc.regions[0].text, visible);
        let patch = doc
            .change_link(source, 0, 1..1, "#new")
            .unwrap_or_else(|_| panic!("edit {source}"));
        let mut changed = source.to_owned();
        changed.replace_range(patch.range, &patch.text);
        let parsed = Markdown::parse(&changed);
        assert_eq!(parsed.regions[0].text, visible);
        assert_eq!(parsed.source_links[0].target, "#new");

        let patch = doc
            .change_link(source, 0, 1..1, "")
            .unwrap_or_else(|_| panic!("remove {source}"));
        let mut removed = source.to_owned();
        removed.replace_range(patch.range, &patch.text);
        let parsed = Markdown::parse(&removed);
        assert_eq!(parsed.regions[0].text, visible);
        assert!(parsed.source_links.is_empty(), "{removed}");
    }
}

#[test]
fn visual_link_edits_stay_inside_a_table_cell_or_a_quoted_paragraph() {
    for (source, needle, old) in [
        (
            "| [label\\|and **bold**](old.md) | Other |\r\n| --- | --- |",
            "bold",
            "old.md",
        ),
        (
            "> [two\r\n> **lines**](old.md \"tip\")\r\n",
            "lines",
            "old.md",
        ),
    ] {
        let doc = Markdown::parse(source);
        let selection = range_for(&doc.regions[0].text, needle);
        let patch = doc
            .change_link(source, 0, selection.clone(), "#new")
            .unwrap_or_else(|error| panic!("{error:?}: {source}"));
        let mut next = source.to_owned();
        next.replace_range(patch.range, &patch.text);
        assert_eq!(next, source.replace(old, "#new"));
        assert!(
            doc.change_link(source, 0, selection, "javascript:alert(1)")
                .is_err()
        );
    }
    let source = "[First](x) [Second](x)";
    let doc = Markdown::parse(source);
    assert!(doc.change_link(source, 0, 0..12, "#new").is_err());
}

#[test]
fn footnote_return_anchors_use_the_first_reference() {
    let source = "# Guide\r\n\r\n[Ordinary link](#fn-details) Actual[^details].\r\n\r\nAgain[^details].\r\n\r\n[^details]: Notes.\r\n";
    let doc = Markdown::parse(source);
    assert_eq!(doc.anchor_byte("fnref-details"), source.find("[^details]"));
    let definition = doc.anchor_byte("fn-details").expect("footnote definition");
    assert!(source[definition..].contains("Notes."));
    assert_eq!(doc.anchor_byte("guide"), Some(0));
    assert!(pane_md::lint::check(source).is_empty());

    let missing = "[Return](#fnref-missing) [Ordinary link](#fn-missing).";
    assert_eq!(Markdown::parse(missing).anchor_byte("fnref-missing"), None);
    assert_eq!(pane_md::lint::check(missing).len(), 2);
}

#[test]
fn destinations_distinguish_web_links_and_local_paths() {
    use pane_md::links::{Destination, TargetError, destination};
    use std::path::PathBuf;

    assert_eq!(
        destination("#%D1%82%D0%B5%D1%81%D1%82").unwrap(),
        Destination::Anchor("тест".into())
    );
    assert_eq!(
        destination("//example.org/guide").unwrap(),
        Destination::External("https://example.org/guide".into())
    );
    for url in [
        "HTTPS://example.org",
        "ftp://example.org",
        "mailto:test@example.org",
        "tel:+123",
        "sms:+123",
    ] {
        assert_eq!(destination(url).unwrap(), Destination::External(url.into()));
    }
    assert_eq!(
        destination("../Документы/с%20пробелом%23.md#раздел").unwrap(),
        Destination::Local {
            path: PathBuf::from("../Документы/с пробелом#.md"),
            anchor: "раздел".into(),
        }
    );
    assert!(matches!(
        destination("javascript:void(0)"),
        Err(TargetError::Unsupported)
    ));
    assert!(matches!(
        destination("https:///"),
        Err(TargetError::Invalid)
    ));
    assert!(matches!(
        destination("file:relative.md"),
        Err(TargetError::Invalid)
    ));
    if cfg!(windows) {
        for url in [
            "file:///C:/docs/a%20b.md#part",
            "file://localhost/C:/docs/a%20b.md#part",
            "C:/docs/a%20b.md#part",
        ] {
            assert_eq!(
                destination(url).unwrap(),
                Destination::Local {
                    path: PathBuf::from("C:/docs/a b.md"),
                    anchor: "part".into()
                },
            );
        }
        assert_eq!(
            destination("file://server/share/guide.md").unwrap(),
            Destination::Local {
                path: PathBuf::from(r"\\server\share\guide.md"),
                anchor: String::new()
            },
        );
        assert!(matches!(
            destination(r"\\?\C:\docs\guide.md"),
            Err(TargetError::Unsupported)
        ));
        assert!(matches!(
            destination("C:/docs/guide.md:other"),
            Err(TargetError::Unsupported)
        ));
    } else {
        assert_eq!(
            destination("file://localhost/home/a%20b.md#part").unwrap(),
            Destination::Local {
                path: PathBuf::from("/home/a b.md"),
                anchor: "part".into()
            },
        );
    }
}

#[test]
fn quoted_and_list_headings_keep_unique_anchors() {
    let source = "# Transition\n\n> ## Example\n\n- ## Example\n\n## Example\n\n[quote](#example) [list](#example-1) [section](#example-2)\n";
    let doc = Markdown::parse(source);
    assert_eq!(
        doc.regions
            .iter()
            .filter(|region| region.level > 0)
            .map(|region| region.slug.as_str())
            .collect::<Vec<_>>(),
        ["transition", "example", "example-1", "example-2"],
    );
    assert!(pane_md::lint::check_parsed(source, &doc).is_empty());
}

#[test]
fn autolinks_can_be_found_by_their_source_coordinates() {
    let source = "One <https://example.org/explicit> two https://example.org/implicit.\n";
    let doc = Markdown::parse(source);
    for (word, target) in [
        ("explicit", "https://example.org/explicit"),
        ("implicit", "https://example.org/implicit"),
    ] {
        let byte = source.find(word).unwrap();
        assert_eq!(
            doc.source_link(byte).map(|link| link.target.as_str()),
            Some(target)
        );
    }
    assert!(doc.source_link(source.find("two").unwrap()).is_none());
}

#[test]
fn task_checkbox_edits_only_the_markdown_marker() {
    let original =
        "# Tasks\r\n\r\n* [X] Parent 😀\r\n  + [ ] **Nested**\r\n\r\n[ ] ordinary text\r\n";
    let doc = Markdown::parse(original);
    let tasks = doc
        .regions
        .iter()
        .filter_map(|r| r.list.as_ref())
        .collect::<Vec<_>>();
    assert_eq!(tasks.len(), 2);
    let (span, replacement) = tasks[1].task_patch(original).unwrap();
    assert_eq!(&original[span.clone()], " ");
    let mut changed = original.to_string();
    changed.replace_range(span, replacement);
    assert_eq!(changed, original.replace("+ [ ]", "+ [x]"));
    let (span, replacement) = tasks[0].task_patch(&changed).unwrap();
    assert_eq!(&changed[span.clone()], "X");
    changed.replace_range(span, replacement);
    assert_eq!(
        changed,
        original.replace("+ [ ]", "+ [x]").replace("* [X]", "* [ ]")
    );
}

#[test]
fn list_continuation_preserves_list_kinds_and_accepts_text_in_the_new_item() {
    for (source, suffix) in [
        ("Prelude\r\n\r\n- Text", "\r\n- "),
        ("07) Text", "\n08) "),
        ("> * [x] **Done**", "\n> * [ ] "),
        ("- **Parent**\n  - [X] Nested", "\n  - [ ] "),
        ("- [x] First line\n  Second line", "\n- [ ] "),
    ] {
        let source_doc = Markdown::parse(source);
        let region = source_doc.regions.last().unwrap();
        let (at, insertion, caret) = region
            .continue_list(source, region.source.end)
            .unwrap_or_else(|| panic!("cannot continue {source:?}"));
        assert_eq!(insertion, suffix, "{source}");
        let mut text = source.to_owned();
        text.replace_range(at, &insertion);
        assert_eq!(caret, text.len());
        let parsed = Markdown::parse(&text);
        let next = parsed.regions.last().unwrap();
        assert!(
            next.list.is_some() && next.source.is_empty(),
            "{text:?}: {:?}",
            next.source
        );
        let local = next.text.encode_utf16().count();
        let (at, inserted) = next.patch(&text, local..local, "Follow-up").unwrap();
        text.replace_range(at, &inserted);
        assert_eq!(text, format!("{source}{suffix}Follow-up"));
    }
}

#[test]
fn plain_list_splits_at_a_scalar_boundary_and_can_exit_at_end_of_file() {
    let source = "- Left😀Right\r\n- [ ] ";
    let parsed = Markdown::parse(source);
    let right = source.find("Right").unwrap();
    assert_eq!(
        parsed.regions[0].continue_list(source, right),
        Some((right..right, "\r\n- ".into(), right + 4))
    );
    assert!(parsed.regions[0].continue_list(source, right - 1).is_none());

    let last = parsed.regions.last().unwrap();
    let (range, replacement, caret) = last.continue_list(source, source.len()).unwrap();
    let mut text = source.to_owned();
    text.replace_range(range, &replacement);
    assert_eq!(text, "- Left😀Right\r\n\r\n");
    assert_eq!(caret, text.len());
}

#[test]
fn leaving_empty_list_keeps_existing_blank_line_endings() {
    let source = "- Previous\r\n- [ ] \n \r\n";
    let parsed = Markdown::parse(source);
    let empty = parsed.regions.last().unwrap();
    let (range, replacement, next) = empty.continue_list(source, empty.source.end).unwrap();
    let mut text = source.to_owned();
    text.replace_range(range, &replacement);
    assert_eq!(text, "- Previous\r\n\n \r\n");
    assert_eq!(next, text.len());
}

#[test]
fn hard_break_keeps_task_text_in_the_same_list_item() {
    let source = "> 12) [x] LeftRight\r\n";
    let parsed = Markdown::parse(source);
    let item = &parsed.regions[0];
    let at = item.text.find("Right").unwrap();
    let at = item.text[..at].encode_utf16().count();
    let (span, replacement) = item.patch(source, at..at, "\u{000b}").unwrap();
    let mut changed = source.to_owned();
    changed.replace_range(span, &replacement);
    assert_eq!(changed, "> 12) [x] Left  \r\n>     Right\r\n");
    let reparsed = Markdown::parse(&changed);
    assert_eq!(reparsed.regions.len(), 1);
    assert_eq!(reparsed.regions[0].text, "☑  Left\u{000b}Right");

    let heading = Markdown::parse("# LeftRight");
    assert!(
        heading.regions[0]
            .patch("# LeftRight", 4..4, "\u{000b}")
            .is_none()
    );
}

#[test]
fn visual_format_keeps_headings_list_tasks_and_table_content() {
    use pane_md::markdown::{BlockFormat, Markdown, TableEdit};
    let source = "# **План** ##\r\n\r\nText\r\n====\r\n\r\n- [x] Done\r\n\r\n| A | B |\r\n| - | - |\r\n| 1 | 2 |\r\n";
    let parsed = Markdown::parse(source);
    let patch = parsed.regions[0]
        .block_format(source, BlockFormat::Heading(2))
        .unwrap();
    assert_eq!(&source[patch.range], "# **План** ##");
    assert_eq!(patch.text, "## **План**");
    let patch = parsed.regions[1]
        .block_format(source, BlockFormat::Bullet)
        .unwrap();
    assert_eq!(patch.text, "- Text");
    let patch = parsed.regions[2]
        .block_format(source, BlockFormat::Numbered)
        .unwrap();
    assert_eq!(&source[patch.range], "-");
    assert_eq!(patch.text, "1.");
    let patch = parsed.edit_table(source, 3, TableEdit::AddColumn).unwrap();
    assert_eq!(
        patch.text,
        "| A |   | B |\r\n| - | --- | - |\r\n| 1 |   | 2 |"
    );
}

#[test]
fn visual_format_toggles_delimiters_without_changing_unicode_text() {
    use pane_md::markdown::{FormatError, InlineFormat, Markdown};
    let source = "Lead __Ж😀__ tail";
    let doc = Markdown::parse(source);
    let patch = doc.regions[0]
        .inline_format(source, 5..8, InlineFormat::Bold)
        .unwrap();
    assert_eq!(&source[patch.range], "__Ж😀__");
    assert_eq!(patch.text, "Ж😀");
    assert_eq!(
        doc.regions[0]
            .inline_format(source, 4..8, InlineFormat::Italic)
            .err(),
        Some(FormatError::Unsupported),
    );
    let patch = doc.regions[0]
        .inline_format(source, 5..8, InlineFormat::Italic)
        .unwrap();
    assert_eq!(patch.text, "*Ж😀*");

    let source = "`**literal**`";
    let doc = Markdown::parse(source);
    assert_eq!(
        doc.regions[0]
            .inline_format(source, 0..11, InlineFormat::Code)
            .err(),
        Some(FormatError::Unsupported),
    );
}

#[test]
fn multiline_paste_preserves_literal_text_paragraphs_and_local_line_endings() {
    use pane_md::markdown::Markdown;

    let source = "# Заголовок\n\nBefore old after\r\n\r\nNext paragraph\n";
    let doc = Markdown::parse(source);
    let patch = doc
        .paste_text(source, &[(1, 7..10)], "Ж😀\r\n- literal\n\n  end")
        .unwrap();
    assert_eq!(&source[patch.range.clone()], "old");
    assert_eq!(patch.text, "Ж😀<br>\\- literal\r\n\r\n&#32;&#32;end");
    let mut updated = source.to_owned();
    updated.replace_range(patch.range, &patch.text);
    assert!(updated.ends_with(" after\r\n\r\nNext paragraph\n"));
    let next = Markdown::parse(&updated);
    assert_eq!(next.regions[1].text, "Before Ж😀\u{000b}- literal");
    assert_eq!(next.regions[2].text, "  end after");
    assert!(updated[..patch.caret.start].ends_with("end"));
}

#[test]
fn visual_paste_keeps_inline_style_links_and_table_columns() {
    use pane_md::markdown::Markdown;

    for (source, expected) in [
        ("**left old right**", "**left one<br>two right**"),
        (
            "[left old right](page.md)",
            "[left one<br>two right](page.md)",
        ),
        ("# left old right", "# left one<br>two right"),
        ("- [x] left old right", "- [x] left one<br>two right"),
        ("> left old right", "> left one<br>two right"),
        (
            "| left old right | B |\n| --- | --- |",
            "| left one<br>two right | B |\n| --- | --- |",
        ),
    ] {
        let doc = Markdown::parse(source);
        let start = doc.regions[0].text.find("old").unwrap();
        let start = doc.regions[0].text[..start].encode_utf16().count();
        let patch = doc
            .paste_text(source, &[(0, start..start + 3)], "one\ntwo")
            .unwrap_or_else(|| panic!("paste failed for {source}"));
        let mut updated = source.to_owned();
        updated.replace_range(patch.range, &patch.text);
        assert_eq!(updated, expected);
        assert!(
            Markdown::parse(&updated).regions[0]
                .text
                .contains("one\u{000b}two")
        );
    }
}

#[test]
fn visual_paste_replaces_whole_inline_spans_or_adjacent_prose() {
    use pane_md::markdown::Markdown;

    let source = "left **old** and *more* right";
    let doc = Markdown::parse(source);
    let patch = doc.paste_text(source, &[(0, 5..17)], "pasted").unwrap();
    assert_eq!(&source[patch.range], "**old** and *more*");
    assert_eq!(patch.text, "pasted");

    let source = "A hello\r\n\r\nGoodbye Z\r\n\r\nAnother";
    let doc = Markdown::parse(source);
    let patch = doc
        .paste_text(source, &[(0, 2..7), (1, 0..7)], "new\ntext")
        .unwrap();
    assert_eq!(&source[patch.range], "hello\r\n\r\nGoodbye");
    assert_eq!(patch.text, "new<br>text");

    let source = "A hello\n\n<!-- Keep this record -->\n\nGoodbye Z";
    let doc = Markdown::parse(source);
    assert!(
        doc.paste_text(source, &[(0, 2..7), (1, 0..7)], "new")
            .is_none()
    );
}

#[test]
fn visual_replacement_can_join_a_heading_and_plain_paragraph() {
    use pane_md::markdown::Markdown;

    for (source, expected) in [
        ("# Plan old\n\nnext text", "# Plan new&#32;text"),
        ("Before old\n\n# Next text", "Before new&#32;text"),
    ] {
        let doc = Markdown::parse(source);
        let left = doc.regions[0].text.find("old").unwrap();
        let right = doc.regions[1].text.find("text").unwrap();
        let patch = doc
            .paste_text(
                source,
                &[(0, left..doc.regions[0].text.len()), (1, 0..right)],
                "new ",
            )
            .unwrap_or_else(|| panic!("failed to join {source}"));
        let mut updated = source.to_owned();
        updated.replace_range(patch.range, &patch.text);
        assert_eq!(updated, expected);
    }

    let source = "# Plan old\n\nnext text";
    let model = Markdown::parse(source);
    let (_, _, anchor) =
        review::capture(&model, 0, range_for(&model.regions[0].text, "old")).unwrap();
    let record = Review {
        v: 1,
        id: "heading-note".into(),
        quote: "old".into(),
        anchor,
        text: "Keep this comment".into(),
        status: Status::Open,
        updated_at: None,
    }
    .encode();
    let source = source.replace("\n\nnext", &format!("\n\n{record}\n\nnext"));
    let model = Markdown::parse(&source);
    let patch = model
        .paste_text(
            &source,
            &[(0, range_for(&model.regions[0].text, "old")), (1, 0..5)],
            "new ",
        )
        .unwrap();
    let mut updated = source.clone();
    updated.replace_range(patch.range, &patch.text);
    assert!(updated.contains(&record));
    assert_eq!(Markdown::parse(&updated).records.len(), 1);
}

#[test]
fn visual_replacement_can_join_a_list_item_and_plain_paragraph() {
    for (source, expected) in [
        ("First old\n\n- [x] next text", "First new&#32;text"),
        ("- [x] First old\n\nnext text", "- [x] First new&#32;text"),
        ("First old\n\n10. next text", "First new&#32;text"),
        ("10. First old\n\nnext text", "10. First new&#32;text"),
    ] {
        let doc = Markdown::parse(source);
        let first = &doc.regions[0].text;
        let last = &doc.regions[1].text;
        let patch = doc
            .paste_text(
                source,
                &[
                    (0, range_for(first, "old")),
                    (1, 0..range_for(last, "text").start),
                ],
                "new ",
            )
            .unwrap_or_else(|| panic!("failed to join {source}"));
        let mut updated = source.to_owned();
        updated.replace_range(patch.range, &patch.text);
        assert_eq!(updated, expected);
    }

    let prose = "First old\n\n- [x] next text";
    let model = Markdown::parse(prose);
    let (_, _, anchor) =
        review::capture(&model, 0, range_for(&model.regions[0].text, "old")).unwrap();
    let record = Review {
        v: 1,
        id: "list-join-note".into(),
        quote: "old".into(),
        anchor,
        text: "Keep this comment".into(),
        status: Status::Open,
        updated_at: None,
    }
    .encode();
    let source = prose.replace("\n\n-", &format!("\n\n{record}\n\n-"));
    let model = Markdown::parse(&source);
    let last = &model.regions[1].text;
    let patch = model
        .paste_text(
            &source,
            &[
                (0, range_for(&model.regions[0].text, "old")),
                (1, 0..range_for(last, "text").start),
            ],
            "new ",
        )
        .unwrap();
    let mut updated = source.clone();
    updated.replace_range(patch.range, &patch.text);
    assert!(updated.contains(&record));
    assert_eq!(Markdown::parse(&updated).records.len(), 1);
}

#[test]
fn visual_replacement_preserves_review_records_between_paragraphs() {
    let first = "Keep **bold** old";
    let last = "Remove *lost* and stay";
    let prose = format!("{first}\r\n\r\n{last}\r\n\r\nUntouched.\r\n");
    let model = Markdown::parse(&prose);
    let record = |region, id: &str, quoted| {
        let (_, quote, anchor) = review::capture(
            &model,
            region,
            range_for(&model.regions[region].text, quoted),
        )
        .unwrap();
        Review {
            v: 1,
            id: id.into(),
            quote,
            anchor,
            text: "Keep this card".into(),
            status: Status::Open,
            updated_at: None,
        }
    };
    let kept = record(0, "kept", "bold");
    let removed = record(1, "removed", "lost");
    let mut source = prose;
    source.insert_str(
        first.len(),
        &format!("\r\n\r\n{}\r\n\r\n{}", kept.encode(), removed.encode()),
    );
    let doc = Markdown::parse(&source);
    let change = doc
        .paste_text(&source, &[(0, 9..13), (1, 0..16)], " then ")
        .expect("replace across selected prose");
    source.replace_range(change.range, &change.text);
    let merged = Markdown::parse(&source);
    assert_eq!(merged.regions[0].text, "Keep bold then stay");
    assert_eq!(merged.regions[1].text, "Untouched.");
    assert!(source.starts_with("Keep **bold**"));
    assert!(source.contains(&format!("{}\r\n\r\n{}", kept.encode(), removed.encode())));
    assert!(matches!(
        merged.records[0].binding,
        Binding::Attached { .. }
    ));
    assert!(matches!(merged.records[1].binding, Binding::Missing));
}

#[test]
fn visual_deletion_removes_whole_formatting_runs_and_joins_paragraphs() {
    let source = "Left **remove** right\n\nNext part.";
    let parsed = Markdown::parse(source);
    let removal = parsed.paste_text(source, &[(0, 5..11)], "").unwrap();
    let mut text = source.to_owned();
    text.replace_range(removal.range, &removal.text);
    assert_eq!(text, "Left  right\n\nNext part.");
    let parsed = Markdown::parse(&text);
    let end = parsed.regions[0].text.encode_utf16().count();
    let join = parsed
        .paste_text(&text, &[(0, end..end), (1, 0..0)], "")
        .unwrap();
    text.replace_range(join.range, &join.text);
    assert_eq!(text, "Left  rightNext part.");
    let whole = Markdown::parse("Delete me.");
    assert_eq!(
        whole
            .paste_text("Delete me.", &[(0, 0..10)], "")
            .unwrap()
            .text,
        ""
    );
}

#[test]
fn visual_replacement_preserves_heading_list_and_cell_structure() {
    for (source, region, input, expected) in [
        (
            "## Keep **old** and *more* tail",
            0,
            "new",
            "## Keep new tail",
        ),
        (
            "- [x] Keep **old** and *more* tail",
            0,
            "",
            "- [x] Keep  tail",
        ),
        (
            "> Keep **old** and *more* tail",
            0,
            "new",
            "> Keep new tail",
        ),
        (
            "| Keep **old** and *more* tail | Other |\n| --- | --- |\n| A | B |",
            0,
            "a|b",
            "| Keep a\\|b tail | Other |\n| --- | --- |\n| A | B |",
        ),
        (
            "| A | B |\n| --- | --- |\n| **old** and *more* | B |",
            2,
            "",
            "| A | B |\n| --- | --- |\n|  | B |",
        ),
    ] {
        let model = Markdown::parse(source);
        let span = range_for(&model.regions[region].text, "old and more");
        let patch = model
            .paste_text(source, &[(region, span)], input)
            .unwrap_or_else(|| panic!("cannot edit {source:?}"));
        let mut actual = source.to_owned();
        actual.replace_range(patch.range, &patch.text);
        assert_eq!(actual, expected);
    }
}

#[test]
fn visual_replacement_joins_sibling_list_items() {
    fn join(source: &str, left: usize, right: usize, start: &str, end: &str) -> Option<String> {
        let model = Markdown::parse(source);
        let length = |region: usize| model.regions[region].text.encode_utf16().count();
        let start = range_for(&model.regions[left].text, start).start;
        let end = range_for(&model.regions[right].text, end).end;
        let mut parts = vec![(left, start..length(left))];
        parts.extend((left + 1..right).map(|region| (region, 0..length(region))));
        parts.push((right, 0..end));
        let patch = model.paste_text(source, &parts, " + ")?;
        let mut actual = source.to_owned();
        actual.replace_range(patch.range, &patch.text);
        Some(actual)
    }
    for (source, left, right, expected) in [
        (
            "- Keep **bold** old\r\n- Remove *this* tail\r\n- After\r\n",
            0,
            1,
            "- Keep **bold**&#32;\\+&#32; tail\r\n- After\r\n",
        ),
        (
            "4. Keep **bold** old\n5. Erase\n6. Remove *this* tail\n7. After\n",
            0,
            2,
            "4. Keep **bold**&#32;\\+&#32; tail\n7. After\n",
        ),
        (
            "- Parent\n  - [x] Keep **bold** old\n  - [ ] Remove *this* tail\n  - [x] After\n",
            1,
            2,
            "- Parent\n  - [x] Keep **bold**&#32;\\+&#32; tail\n  - [x] After\n",
        ),
    ] {
        assert_eq!(
            join(source, left, right, " old", "Remove this").as_deref(),
            Some(expected)
        );
    }
    assert_eq!(
        join(
            "- Keep old\n  - Child\n- Remove this tail\n",
            0,
            2,
            " old",
            "Remove this"
        )
        .as_deref(),
        Some("- Keep&#32;\\+&#32; tail\n"),
    );
    assert!(
        join(
            "- A\n  - Keep old\n- B\n  - Remove this tail\n",
            1,
            3,
            " old",
            "Remove this"
        )
        .is_none()
    );
}

#[test]
fn visual_replacement_across_table_cells_preserves_the_grid() {
    let table = "| Keep **bold** old | More |\r\n| :--- | ---: |\r\n| | Drop *this* and stay |\r\n| Keep | **After** |\r\n";
    let model = Markdown::parse(table);
    let card = |id: &str, region: usize, text| {
        let (_, quote, anchor) =
            review::capture(&model, region, range_for(&model.regions[region].text, text)).unwrap();
        Review {
            v: 1,
            id: id.into(),
            quote,
            anchor,
            text: "Check this wording".into(),
            status: Status::Open,
            updated_at: None,
        }
        .encode()
    };
    let comments = format!(
        "\r\n{}\r\n\r\n{}\r\n",
        card("selected-cell", 3, "Drop this"),
        card("untouched-cell", 5, "After")
    );
    let source = format!("{table}{comments}");
    let doc = Markdown::parse(&source);
    let first = &doc.regions[0].text;
    let last = &doc.regions[3].text;
    let selection = [
        (0, range_for(first, " old")),
        (1, 0..4),
        (2, 0..0),
        (3, 0..range_for(last, "Drop this").end),
    ];
    let patch = doc
        .paste_text(&source, &selection, " ⇢ ")
        .expect("edit across rows and cells");
    let mut text = source.clone();
    text.replace_range(patch.range, &patch.text);
    assert_eq!(
        text.strip_suffix(&comments).unwrap(),
        "| Keep **bold**&#32;⇢&#32; |  |\r\n| :--- | ---: |\r\n| |  and stay |\r\n| Keep | **After** |\r\n"
    );
    let updated = Markdown::parse(&text);
    assert!(matches!(updated.records[0].binding, Binding::Missing));
    assert!(matches!(
        updated.records[1].binding,
        Binding::Attached { .. }
    ));
    assert!(
        doc.paste_text(&source, &[selection[0].clone(), selection[3].clone()], "")
            .is_none()
    );
}

#[test]
fn visual_paste_accepts_literal_fenced_code_and_keeps_the_fence() {
    use pane_md::markdown::Markdown;

    let source = "```txt\r\nkeep\r\n\r\n```";
    let doc = Markdown::parse(source);
    let patch = doc
        .paste_text(source, &[(0, 5..5)], "raw *a*\nsecond")
        .unwrap();
    let mut updated = source.to_owned();
    updated.replace_range(patch.range, &patch.text);
    assert_eq!(updated, "```txt\r\nkeep\r\nraw *a*\r\nsecond\r\n```");
    assert!(
        doc.paste_text(source, &[(0, 5..5)], "```\n# Heading")
            .is_none()
    );
}

#[test]
fn pasted_markup_stays_literal_and_the_existing_review_record_stays_attached() {
    let prose = "# Draft\n\nKeep reviewed text.\n\n| Target | Other |\n| --- | --- |\n";
    let original = Markdown::parse(prose);
    let (after, quote, anchor) = review::capture(
        &original,
        1,
        range_for(&original.regions[1].text, "reviewed"),
    )
    .unwrap();
    let record = Review {
        v: 1,
        id: "paste-review".into(),
        quote,
        anchor,
        text: "Preserve this".into(),
        status: Status::Open,
        updated_at: None,
    };
    let mut source = prose.to_owned();
    source.insert_str(after, &review::insertion(prose, after, &record));
    let model = Markdown::parse(&source);
    let cell = model
        .regions
        .iter()
        .position(|r| r.kind == "table")
        .unwrap();
    let patch = model
        .paste_text(&source, &[(cell, 0..6)], "a|b\n<img src=x>")
        .unwrap();
    source.replace_range(patch.range, &patch.text);
    assert!(source.contains("a\\|b<br>\\<img src\\=x\\>"));
    assert!(source.contains(&record.encode()));
    let reparsed = Markdown::parse(&source);
    assert_eq!(reparsed.regions[cell].text, "a|b\u{000b}<img src=x>");
    assert!(matches!(
        reparsed.records[0].binding,
        Binding::Attached { .. }
    ));
    assert!(source[..source.find("| a").unwrap()].contains("Keep reviewed text."));
}

#[test]
fn literal_search_handles_unicode_whole_words_and_case_without_regex_syntax() {
    use pane_md::search::{Options, matches};
    let source = "Жук жук жуки Жук_1 (ЖУК) [Жук]";
    let plain = Options {
        case_sensitive: false,
        whole_word: true,
    };
    let hits = matches(source, "жук", plain);
    assert_eq!(
        hits.iter()
            .map(|span| &source[span.clone()])
            .collect::<Vec<_>>(),
        ["Жук", "жук", "ЖУК", "Жук"]
    );
    let sensitive = Options {
        case_sensitive: true,
        ..plain
    };
    assert_eq!(matches(source, "жук", sensitive).len(), 1);
    assert_eq!(
        &source[matches(source, "[Жук]", Options::default())[0].clone()],
        "[Жук]"
    );
    assert!(matches(source, "", plain).is_empty());
}

#[test]
fn source_replace_preserves_protected_records_and_line_endings_in_one_patch() {
    use pane_md::search::{Options, replace};
    let mut source = String::from(
        "old\r\n\r\n<!-- pane-md:review invalid old record -->\n<!-- notepad-md:review invalid old record -->\n\nold\n\n<!-- authored old -->\n",
    );
    let edit = replace(&source, "old", "new\nlines", Options::default(), None).unwrap();
    assert_eq!(edit.count, 3);
    source.replace_range(edit.range, &edit.text);
    assert_eq!(
        source,
        "new\r\nlines\r\n\r\n<!-- pane-md:review invalid old record -->\n<!-- notepad-md:review invalid old record -->\n\nnew\nlines\n\n<!-- authored new\nlines -->\n"
    );
    assert!(source[..edit.caret].ends_with("lines"));
    let from = "A old B old";
    let second = replace(from, "old", "X", Options::default(), Some(8..11)).unwrap();
    assert_eq!(second.range, 8..11);
    assert_eq!(second.text, "X");
}
