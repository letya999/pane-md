use std::{
    collections::{HashMap, HashSet},
    fs, io,
    path::Path,
};

use serde::Serialize;

use crate::{
    links::{Destination, TargetError, decode_percent, destination, markdown_path},
    markdown::{Lines, Markdown},
    review::Binding,
    text_file::decode,
};

#[derive(Debug, PartialEq, Eq, Serialize)]
pub struct Issue {
    pub code: &'static str,
    pub line: usize,
    pub column: usize,
    pub message: String,
}

/// Lints without modifying the source or opening any link targets.
pub fn check(source: &str) -> Vec<Issue> {
    check_parsed(source, &Markdown::parse(source))
}

/// Lints the same source text from which the supplied document was parsed.
pub fn check_parsed(source: &str, doc: &Markdown) -> Vec<Issue> {
    check_document(source, doc, None)
}

/// Lints a file and checks local destinations relative to its directory.
pub fn check_at(source: &str, path: &Path) -> Vec<Issue> {
    check_parsed_at(source, &Markdown::parse(source), path)
}

/// Like [check_at], with the parsed snapshot of the supplied source.
pub fn check_parsed_at(source: &str, doc: &Markdown, path: &Path) -> Vec<Issue> {
    check_document(source, doc, Some(path))
}

fn check_document(source: &str, doc: &Markdown, path: Option<&Path>) -> Vec<Issue> {
    let lines = Lines::new(source);
    let mut issues = doc
        .diagnostics
        .iter()
        .map(|diagnostic| Issue {
            code: "review-record",
            line: diagnostic.line,
            column: 1,
            message: diagnostic.message.clone(),
        })
        .collect::<Vec<_>>();
    let mut issue = |code, at, message| {
        let (line, column) = lines.position(at);
        issues.push(Issue {
            code,
            line,
            column,
            message,
        });
    };

    for record in &doc.records {
        let (code, detail) = match record.binding {
            Binding::Attached { .. } => continue,
            Binding::Missing => ("review-anchor-missing", "is missing"),
            Binding::Ambiguous => ("review-anchor-ambiguous", "matches several places"),
        };
        issue(
            code,
            record.source.start,
            format!("anchor for review {} {detail}", record.review.id),
        );
    }

    let anchors = doc
        .regions
        .iter()
        .filter_map(|region| (!region.slug.is_empty()).then_some(region.slug.as_str()))
        .collect::<HashSet<_>>();
    let links = doc
        .source_links
        .iter()
        .filter(|link| link.footnote)
        .filter_map(|link| link.target.strip_prefix("#fn-"))
        .collect::<HashSet<_>>();
    for region in &doc.regions {
        let mut checked = HashSet::new();
        for run in &region.runs {
            let Some(fragment) = run.link.as_deref().and_then(|url| url.strip_prefix('#')) else {
                continue;
            };
            if fragment.is_empty() || !checked.insert(fragment) {
                continue;
            }
            let at = run
                .units
                .iter()
                .flatten()
                .next()
                .map_or(region.source.start, |range| range.start);
            match decode_percent(fragment) {
                Ok(id)
                    if anchors.contains(id.as_str())
                        || id
                            .strip_prefix("fnref-")
                            .is_some_and(|name| links.contains(name)) => {}
                Ok(id) => issue("anchor-missing", at, format!("no heading matches #{id}")),
                Err(_) => issue(
                    "anchor-encoding",
                    at,
                    format!("invalid anchor encoding in #{fragment}"),
                ),
            }
        }
    }
    if let Some(path) = path {
        check_local_targets(doc, path, &mut issue);
    }
    issues.sort_by_key(|issue| (issue.line, issue.column, issue.code));
    issues
}

fn check_local_targets(
    doc: &Markdown,
    path: &Path,
    issue: &mut impl FnMut(&'static str, usize, String),
) {
    let current = path.canonicalize().ok();
    let directory = current
        .as_deref()
        .unwrap_or(path)
        .parent()
        .unwrap_or_else(|| Path::new(""));
    let mut targets = HashMap::new();
    let mut documents = HashMap::new();
    let links = doc
        .source_links
        .iter()
        .map(|link| (link.source.start, link.target.as_str(), link.image));
    let images = doc
        .nested_images
        .iter()
        .map(|(at, target)| (*at, target.as_str(), true));

    for (at, raw, image) in links.chain(images) {
        if raw.starts_with('#') || image && raw.starts_with("data:") {
            continue;
        }
        let (target, anchor) = match destination(raw) {
            Ok(Destination::Local { path, anchor }) => (path, anchor),
            Err(TargetError::Decode(_)) => {
                issue(
                    "local-target-encoding",
                    at,
                    format!("invalid encoding in local target {raw}"),
                );
                continue;
            }
            _ => continue,
        };
        let candidate = if target.is_absolute() {
            target
        } else {
            directory.join(target)
        };
        let resolved = targets.entry(candidate).or_insert_with_key(|path| {
            let resolved = path.canonicalize()?;
            let metadata = fs::metadata(&resolved)?;
            Ok::<_, io::Error>((resolved, metadata))
        });
        let (target, metadata) = match resolved {
            Ok(value) => value,
            Err(error)
                if matches!(
                    error.kind(),
                    io::ErrorKind::NotFound | io::ErrorKind::NotADirectory
                ) =>
            {
                issue(
                    "local-target-missing",
                    at,
                    format!("local target does not exist: {raw}"),
                );
                continue;
            }
            Err(error) => {
                issue(
                    "local-target-unreadable",
                    at,
                    format!("could not inspect {raw}: {error}"),
                );
                continue;
            }
        };
        if !(metadata.is_file() || !image && metadata.is_dir()) {
            issue(
                "local-target-unreadable",
                at,
                format!("unsupported local target type: {raw}"),
            );
            continue;
        }
        if image || anchor.is_empty() || !metadata.is_file() || !markdown_path(target) {
            continue;
        }
        let linked = if Some(target.as_path()) == current.as_deref() {
            Some(doc)
        } else {
            documents
                .entry(target.clone())
                .or_insert_with_key(|target| {
                    let bytes = fs::read(target).ok()?;
                    let (_, source) = decode(&bytes).ok()?;
                    Some(Markdown::parse(source))
                })
                .as_ref()
        };
        match linked {
            Some(linked) if linked.anchor_byte(&anchor).is_none() => {
                issue(
                    "anchor-missing",
                    at,
                    format!("no heading matches #{anchor} in {raw}"),
                );
            }
            None => issue(
                "local-target-unreadable",
                at,
                format!("could not read Markdown target: {raw}"),
            ),
            _ => {}
        }
    }
}
