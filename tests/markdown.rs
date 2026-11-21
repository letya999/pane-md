use notepad_md::{
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
    };
    let encoded = record.encode();
    assert_eq!(encoded.matches("-->").count(), 1);
    assert!(encoded.contains("\\u002D\\u002D>"));
    assert!(!encoded.contains('\n'));
    let mut source = before.to_owned();
    source.insert_str(after, &review::insertion(before, after, &record));
    assert_eq!(&source[..after], &before[..after]);
    assert!(source[after..].starts_with("\n\n<!-- notepad-md:review "));
    let doc = Markdown::parse(&source);
    assert!(doc.diagnostics.is_empty());
    assert_eq!(doc.records[0].review, record);
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
        "```md\n{encoded}\n```\n\n{source}\n<!-- notepad-md:review {{broken}} -->\n\n<!-- notepad-md:review {{\"v\":99}} -->\n"
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
