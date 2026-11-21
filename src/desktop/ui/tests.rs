use super::*;

fn index(text: &[u16], needle: &str) -> Range<usize> {
    let units = needle.encode_utf16().collect::<Vec<_>>();
    let at = text
        .windows(units.len())
        .position(|window| window == units)
        .expect("rendered text");
    at..at + units.len()
}

#[test]
fn rich_edit_native_round_trip() -> Result<()> {
    unsafe {
        CoInitializeEx(None, COINIT_APARTMENTTHREADED).ok()?;
        LoadLibraryW(w!("msftedit.dll"))?;
        InitCommonControlsEx(&INITCOMMONCONTROLSEX {
            dwSize: size_of::<INITCOMMONCONTROLSEX>() as u32,
            dwICC: ICC_TREEVIEW_CLASSES | ICC_TAB_CLASSES,
        })
        .ok()
        .map_err(|e| format!("InitCommonControlsEx: {e}"))?;
        let instance: HINSTANCE = GetModuleHandleW(None)?.into();
        let parent = CreateWindowExW(
            WINDOW_EX_STYLE(0),
            w!("STATIC"),
            w!("notepad-md test"),
            WS_OVERLAPPEDWINDOW,
            100,
            100,
            900,
            700,
            None,
            None,
            Some(instance),
            None,
        )?;
        let mut app = App::new(parent, instance, false)?;
        let files = tempfile::tempdir()?;
        let path = files.path().join("Мой документ.md");
        let nested = files.path().join("docs");
        fs::create_dir(&nested)?;
        fs::write(nested.join("guide.md"), "# Начало\n\n## Якорь\n\nЗдесь.")?;
        let rendered = "\
---\r\nowner: team\r\n---\r\n\r\n# Lead time\r\n\r\nKeep **жирный** and _spacing_. [Перейти](docs/guide.md#якорь)\r\n\r\n\
| A | B | C | D | E |\r\n\
| --- | --- | --- | --- | --- |\r\n\
| P85 | Очень длинная ячейка с переносом и несколькими словами. Очень длинная ячейка с переносом. | [ссылка](docs/guide.md#якорь) | Ж😀 | конец |\r\n\
\r\nKeep **after**.\r\n\r\n- [x] Done\r\n- [ ] Next\r\n\r\n<!-- обычный -->\r\n\r\n\
~~отменено~~, `code`, &amp; и жёсткая  \r\nстрока со сноской[^first].\r\n\r\n\
[^first]: Сноска.\r\n\r\n```md\r\nkeep  spacing\r\n```\r\n";
        let mut original = b"\xef\xbb\xbf".to_vec();
        original.extend_from_slice(rendered.as_bytes());
        fs::write(&path, &original)?;
        app.open(files.path().to_path_buf(), 0)?;
        assert!(app.tree_open);
        let first = app
            .tree_entries
            .iter()
            .position(|entry| entry.path.ends_with("docs"))
            .unwrap();
        let root = send(app.tree, TVM_GETNEXTITEM, TVGN_ROOT as usize, 0);
        let child = send(app.tree, TVM_GETNEXTITEM, TVGN_CHILD as usize, root);
        app.expand(HTREEITEM(child), first);
        assert!(
            app.tree_entries
                .iter()
                .any(|entry| entry.path.ends_with("guide.md"))
        );
        app.open(path.clone(), 0)?;
        assert!(
            app.projection_safe,
            "rendered projection mismatch: {:?}",
            String::from_utf16_lossy(&buffer(app.editor))
        );
        let rich_before = buffer(app.editor);
        let span = index(&rich_before, "P85");
        select(app.editor, span.clone(), false);
        send(app.editor, WM_CHAR, 'Z' as usize, 0);
        assert_eq!(buffer(app.editor), rich_before);
        app.follow_at(index(&rich_before, "Перейти").start);
        assert_eq!(app.docs.len(), 2);
        assert_eq!(
            app.docs[app.active.unwrap()].path.file_name().unwrap(),
            "guide.md"
        );
        assert_ne!(selection(app.editor).start, 0);
        app.command(BACK);
        assert_eq!(app.active, Some(0));

        app.command(MODES + 1);
        assert!(app.projection_safe, "source projection at EOF");
        let source_before = buffer(app.editor);
        send(app.editor, WM_CHAR, 'Z' as usize, 0);
        assert_eq!(buffer(app.editor), source_before);
        assert_eq!(fs::read(&path)?, original);

        app.command(MODES + 2);
        fn type_selection(app: &mut App, old: &str, new: &str) {
            let span = index(&buffer(app.editor), old);
            select(app.editor, span, false);
            let replacement = wide(new);
            send(app.editor, EM_REPLACESEL, 1, replacement.as_ptr() as isize);
            app.sync();
        }
        type_selection(&mut app, "жирный", "акцент");
        type_selection(&mut app, "конец", "итог");
        app.command(MODES + 1);
        assert!(String::from_utf16_lossy(&buffer(app.editor)).contains("**акцент**"));
        app.command(MODES + 3);
        type_selection(&mut app, "_spacing_", "_style_");
        app.command(MODES);
        let modified = rendered
            .replace("жирный", "акцент")
            .replace("конец", "итог")
            .replace("_spacing_", "_style_");
        assert_eq!(app.docs[0].text, modified);
        app.command(FIND);
        set_text(app.find_box, "p85");
        app.command(FIND_NEXT);
        let span = selection(app.editor);
        assert_eq!(String::from_utf16_lossy(&buffer(app.editor)[span]), "P85");
        app.command(ADD_REVIEW);
        assert!(app.review_open && app.draft.is_some());
        set_text(app.draft_box, "Комментарий Ж😀 -->");
        app.command(REVIEW_SAVE);
        assert_eq!(app.parsed.records[0].review.quote, "P85");
        assert!(matches!(
            app.parsed.records[0].binding,
            Binding::Attached {
                row: Some(1),
                cell: Some(0),
                ..
            }
        ));
        let record = app.parsed.records[0].source.clone();
        assert!(app.docs[0].text[record.clone()].contains("\\u002D\\u002D>"));
        assert_eq!(buffer(app.editor), app.view.text);
        app.docs[0].save()?;
        let on_disk = fs::read(&path)?;
        assert!(on_disk.starts_with(b"\xef\xbb\xbf"));
        let mut restored = String::from_utf8(on_disk[3..].to_vec())?;
        restored.replace_range(record.start - 4..record.end, "");
        assert_eq!(restored, modified);
        app.command(MODES + 3);
        type_selection(&mut app, "_style_", "_next_");
        fs::write(&path, "external write")?;
        assert!(app.docs[0].save().is_err());
        assert!(app.docs[0].dirty());
        assert!(app.docs[0].text.contains("_next_"));
        assert_eq!(fs::read_to_string(&path)?, "external write");

        let picture = files.path().join("pixel.png");
        fs::write(
            &picture,
            [
                0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a, 0x00, 0x00, 0x00, 0x0d, 0x49, 0x48,
                0x44, 0x52, 0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x01, 0x08, 0x06, 0x00, 0x00,
                0x00, 0x1f, 0x15, 0xc4, 0x89, 0x00, 0x00, 0x00, 0x0d, 0x49, 0x44, 0x41, 0x54, 0x08,
                0x1d, 0x63, 0xf8, 0xcf, 0xc0, 0xf0, 0x1f, 0x00, 0x05, 0x80, 0x02, 0x3f, 0x49, 0xc2,
                0xed, 0xb4, 0x00, 0x00, 0x00, 0x00, 0x49, 0x45, 0x4e, 0x44, 0xae, 0x42, 0x60, 0x82,
            ],
        )?;
        let image_doc = files.path().join("image.md");
        fs::write(
            &image_doc,
            "Before ![local](pixel.png) after. ![remote](https://example.invalid/no.png)",
        )?;
        app.open(image_doc, 0)?;
        assert!(app.projection_safe);
        assert_eq!(
            buffer(app.editor)
                .iter()
                .filter(|&&unit| unit == 0xfffc)
                .count(),
            1
        );
        assert!(String::from_utf16_lossy(&buffer(app.editor)).contains("[remote]"));
        let span = index(&buffer(app.editor), "after");
        select(app.editor, span, false);
        assert_eq!(app.capture_selection().unwrap().1, "after");

        let typing = files.path().join("typing.md");
        fs::write(&typing, "Hello")?;
        app.open(typing, 2)?;
        let pos = buffer(app.editor).len();
        select(app.editor, pos..pos, false);
        for ch in " world".chars() {
            send(app.editor, WM_CHAR, ch as usize, 1);
            app.sync();
        }
        assert_eq!(app.docs[app.active.unwrap()].text, "Hello world");
        app.command(PARAGRAPH);
        send(app.editor, WM_CHAR, 'A' as usize, 1);
        app.sync();
        assert_eq!(app.docs[app.active.unwrap()].text, "Hello world\n\nA");
        DestroyWindow(parent)?;
        CoUninitialize();
        Ok(())
    }
}
