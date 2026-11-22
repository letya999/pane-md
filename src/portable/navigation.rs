use pane_md::{
    Bytes,
    markdown::{Markdown, Region},
};

use super::{Workspace, format, reviews::Part, source_byte};
use crate::i18n::{Label as L, tr};

impl Workspace {
    pub(super) fn find_heading(
        &mut self,
        query: &str,
        previous: bool,
        caret: usize,
        region: Option<usize>,
    ) {
        let Some(doc) = self.active_doc() else { return };
        let parsed = format::projection(doc);
        let current = if doc.mode % 2 == 1 {
            Some(source_byte(&doc.text, caret))
        } else {
            region
                .and_then(|index| parsed.regions.get(index))
                .map(|block| block.source.start)
        };
        let query = query.to_lowercase();
        let headings = || {
            parsed
                .regions
                .iter()
                .filter(|block| block.level > 0 && block.text.to_lowercase().contains(&query))
        };
        let heading = if previous {
            headings()
                .rfind(|block| current.is_none_or(|at| block.source.start < at))
                .or_else(|| headings().next_back())
        } else {
            headings()
                .find(|block| current.is_none_or(|at| block.source.start > at))
                .or_else(|| headings().next())
        };
        self.jump = heading.map(|block| block.source.start);
        if heading.is_none() {
            self.notice = tr(L::NoHeadings).into();
        }
    }

    pub(super) fn show_in_source(&mut self, region: Option<usize>, selection: Option<[Part; 2]>) {
        let Some(doc) = self.active_doc().filter(|doc| doc.mode.is_multiple_of(2)) else {
            return;
        };
        let parsed = format::projection(doc);
        let span = selection
            .and_then(|parts| source_span(&parsed, parts))
            .or_else(|| {
                let block = parsed.regions.get(region?)?;
                Some(if block.level > 0 {
                    block.source.clone()
                } else {
                    block.source.start..block.source.start
                })
            })
            .unwrap_or(0..0);
        doc.mode |= 1;
        self.jump = Some(span.start);
        self.jump_end = Some(span.end);
    }
}

fn source_span(parsed: &Markdown, [first, last]: [Part; 2]) -> Option<Bytes> {
    let (start_index, start) = first.checked(parsed)?;
    let (end_index, end) = last.checked(parsed)?;
    if start_index > end_index || start_index == end_index && start != end {
        return None;
    }
    let first = &parsed.regions[start_index];
    let last = &parsed.regions[end_index];
    let left = first
        .source_selection(start.clone())
        .map_or_else(|| source_caret(first, start.start), |span| span.start);
    let right = last
        .source_selection(end.clone())
        .map_or_else(|| source_caret(last, end.end), |span| span.end);
    (left <= right).then_some(left..right)
}

fn source_caret(block: &Region, offset: usize) -> usize {
    let mut units = block.runs.iter().flat_map(|run| &run.units);
    let left = units
        .by_ref()
        .take(offset)
        .filter_map(Option::as_ref)
        .last();
    units
        .find_map(Option::as_ref)
        .map(|span| span.start)
        .or_else(|| left.map(|span| span.end))
        .unwrap_or(block.source.start)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::{from_value, json};
    use std::fs;

    fn part(region: usize, start: usize, end: usize, text: &str) -> Part {
        from_value(json!({"region": region, "start": start, "end": end, "text": text})).unwrap()
    }

    #[test]
    fn headings_skip_body_text_wrap_and_include_unsaved_markdown() -> crate::Result<()> {
        let directory = tempfile::tempdir()?;
        let path = directory.path().join("Navigation.md");
        fs::write(&path, "# План A\r\n\r\nПлан в абзаце.\r\n\r\n## План B\r\n")?;
        let mut workspace = Workspace::default();
        workspace.open(path, 0)?;
        workspace.find_heading("пЛАН", false, 0, Some(0));
        let text = &workspace.docs[0].text;
        assert_eq!(workspace.jump, text.find("## План B"));
        workspace.find_heading("план", false, 0, Some(2));
        assert_eq!(workspace.jump, Some(0));
        workspace.find_heading("План", true, 0, Some(0));
        assert_eq!(workspace.jump, workspace.docs[0].text.find("## План B"));

        let doc = &mut workspace.docs[0];
        doc.mode = 3;
        Workspace::apply_edits(
            doc,
            Some("# План A\n\nПлан в абзаце.\n\n## План B\n\n### План C\n".into()),
            vec![],
        )?;
        workspace.find_heading("план c", false, 0, None);
        let doc = &workspace.docs[0];
        assert_eq!(workspace.jump, doc.text.find("### План C"));
        assert!(doc.dirty());
        workspace.find_heading("absent", true, 0, None);
        assert_eq!(workspace.jump, None);
        Ok(())
    }

    #[test]
    fn visual_source_navigation_keeps_stage_and_uses_exact_utf16_selection() -> crate::Result<()> {
        let directory = tempfile::tempdir()?;
        let path = directory.path().join("Navigation.md");
        let original = "One **😀 red** tail.\r\n\r\nLast block.\r\n\r\n## Target\r\n";
        fs::write(&path, original)?;
        let mut workspace = Workspace::default();
        workspace.open(path, 2)?;
        workspace.show_in_source(
            Some(0),
            Some([part(0, 4, 10, "😀 red"), part(1, 0, 4, "Last")]),
        );
        let doc = &workspace.docs[0];
        assert_eq!(doc.mode, 3);
        assert!(!doc.dirty());
        assert_eq!(
            &doc.text[workspace.jump.unwrap()..workspace.jump_end.unwrap()],
            "😀 red** tail.\r\n\r\nLast"
        );

        let doc = &mut workspace.docs[0];
        doc.mode = 0;
        doc.show_reviews = false;
        workspace.show_in_source(Some(0), Some([part(0, 6, 6, ""), part(0, 6, 6, "")]));
        let doc = &workspace.docs[0];
        assert_eq!(doc.mode, 1);
        assert!(!doc.show_reviews);
        let caret = original.find(" red").unwrap();
        assert_eq!(
            (workspace.jump, workspace.jump_end),
            (Some(caret), Some(caret))
        );
        workspace.docs[0].mode = 0;
        workspace.show_in_source(Some(2), None);
        let block = &workspace.docs[0].text[workspace.jump.unwrap()..workspace.jump_end.unwrap()];
        assert_eq!(block, "## Target");
        Ok(())
    }
}
