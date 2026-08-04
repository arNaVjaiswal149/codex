//! Opportunistic Mermaid rendering for finalized assistant Markdown.
//!
//! Mermaid source always remains the fallback: complete top-level fences and known raw diagram
//! headers with indented bodies are considered, and one failure does not affect later blocks.

use std::time::Duration;
use std::time::Instant;

use anyhow::Result;
use anyhow::bail;

use crate::terminal_hyperlinks::HyperlinkLine;

const MAX_SOURCE_BYTES: usize = 128 * 1024;
const MAX_DIAGRAMS_PER_MESSAGE: usize = 12;
const TOTAL_RENDER_TIMEOUT: Duration = Duration::from_secs(30);

#[derive(Debug, Clone, PartialEq, Eq)]
enum MarkdownSegment<'a> {
    Markdown(&'a str),
    Mermaid { source: &'a str, diagram: &'a str },
}

#[derive(Debug, Clone, Copy)]
struct OpenFence {
    block_start: usize,
    diagram_start: usize,
    marker: char,
    marker_len: usize,
    is_mermaid: bool,
}

#[derive(Debug, Clone, Copy)]
struct FenceLine<'a> {
    marker: char,
    marker_len: usize,
    remainder: &'a str,
}

#[derive(Debug, Clone, Copy)]
struct RawBlock {
    block_start: usize,
    diagram_start: usize,
    indent: usize,
    has_body: bool,
}

/// Render complete top-level Mermaid fences as Kitty image placeholders when supported.
pub(crate) fn render_markdown_with_mermaid(
    markdown: &str,
    width: Option<usize>,
    feature_enabled: bool,
    render_markdown: impl Fn(&str) -> Vec<HyperlinkLine>,
) -> Vec<HyperlinkLine> {
    if !mermaid_rendering_enabled(feature_enabled) || !contains_mermaid_diagram(markdown) {
        return render_markdown(markdown);
    }
    render_markdown_with_mermaid_using(markdown, width, &render_markdown, render_mermaid)
}

/// Whether Markdown contains an eligible fenced or indented raw Mermaid diagram.
pub(crate) fn contains_mermaid_diagram(markdown: &str) -> bool {
    parse_mermaid_blocks(markdown)
        .iter()
        .any(|segment| matches!(segment, MarkdownSegment::Mermaid { .. }))
}

pub(crate) fn render_markdown_with_mermaid_using<R>(
    markdown: &str,
    width: Option<usize>,
    render_markdown: &dyn Fn(&str) -> Vec<HyperlinkLine>,
    render_mermaid: R,
) -> Vec<HyperlinkLine>
where
    R: Fn(&str, usize) -> Result<Vec<HyperlinkLine>>,
{
    let mut lines = Vec::new();
    let started = Instant::now();
    let mut rendered_diagrams = 0;
    for segment in parse_mermaid_blocks(markdown) {
        match segment {
            MarkdownSegment::Markdown(source) => lines.extend(render_markdown(source)),
            MarkdownSegment::Mermaid { source, diagram } => {
                if rendered_diagrams >= MAX_DIAGRAMS_PER_MESSAGE
                    || started.elapsed() >= TOTAL_RENDER_TIMEOUT
                {
                    lines.extend(render_markdown(source));
                    continue;
                }
                let Some(width) = width.filter(|width| *width > 0) else {
                    lines.extend(render_markdown(source));
                    continue;
                };
                rendered_diagrams += 1;
                match render_mermaid(diagram, width) {
                    Ok(rendered) => lines.extend(rendered),
                    Err(err) => {
                        tracing::debug!(error = %err, "Mermaid rendering unavailable");
                        lines.extend(render_markdown(source));
                    }
                }
            }
        }
    }
    lines
}

fn mermaid_rendering_enabled(feature_enabled: bool) -> bool {
    crate::latex_render::kitty_graphics_enabled("CODEX_MERMAID_RENDER", feature_enabled)
}

pub(crate) fn mermaid_rendering_requested(feature_enabled: bool) -> bool {
    crate::latex_render::rendering_requested("CODEX_MERMAID_RENDER", feature_enabled)
}

fn parse_mermaid_blocks(markdown: &str) -> Vec<MarkdownSegment<'_>> {
    let mut segments = Vec::new();
    let mut cursor = 0;
    let mut emitted = 0;
    let mut open: Option<OpenFence> = None;
    let mut raw: Option<RawBlock> = None;

    for line in markdown.split_inclusive('\n') {
        let line_start = cursor;
        cursor += line.len();
        let content = line.strip_suffix('\n').unwrap_or(line);
        let content = content.strip_suffix('\r').unwrap_or(content);
        if let Some(raw_block) = raw.as_mut() {
            let indent = content.len() - content.trim_start_matches(' ').len();
            if content.trim().is_empty() {
                continue;
            }
            if indent > raw_block.indent && fence_line(content.trim_start()).is_none() {
                raw_block.has_body = true;
                continue;
            }
            push_raw_segment(
                markdown,
                &mut segments,
                &mut emitted,
                *raw_block,
                line_start,
            );
            raw = None;
        }
        if let Some(open_fence) = open {
            if is_closing_fence(content, open_fence.marker, open_fence.marker_len) {
                if open_fence.is_mermaid {
                    if emitted < open_fence.block_start {
                        segments.push(MarkdownSegment::Markdown(
                            &markdown[emitted..open_fence.block_start],
                        ));
                    }
                    segments.push(MarkdownSegment::Mermaid {
                        source: &markdown[open_fence.block_start..cursor],
                        diagram: &markdown[open_fence.diagram_start..line_start],
                    });
                    emitted = cursor;
                }
                open = None;
            }
        } else if let Some(fence) = fence_line(content) {
            open = Some(OpenFence {
                block_start: line_start,
                diagram_start: cursor,
                marker: fence.marker,
                marker_len: fence.marker_len,
                is_mermaid: fence.remainder.trim().eq_ignore_ascii_case("mermaid"),
            });
        } else if let Some(indent) = raw_mermaid_indent(content) {
            raw = Some(RawBlock {
                block_start: line_start,
                diagram_start: line_start + indent,
                indent,
                has_body: false,
            });
        }
    }
    if let Some(raw_block) = raw {
        push_raw_segment(
            markdown,
            &mut segments,
            &mut emitted,
            raw_block,
            markdown.len(),
        );
    }
    if emitted < markdown.len() {
        segments.push(MarkdownSegment::Markdown(&markdown[emitted..]));
    }
    if segments.is_empty() {
        segments.push(MarkdownSegment::Markdown(markdown));
    }
    segments
}

fn push_raw_segment<'a>(
    markdown: &'a str,
    segments: &mut Vec<MarkdownSegment<'a>>,
    emitted: &mut usize,
    raw: RawBlock,
    end: usize,
) {
    if !raw.has_body {
        return;
    }
    if *emitted < raw.block_start {
        segments.push(MarkdownSegment::Markdown(
            &markdown[*emitted..raw.block_start],
        ));
    }
    segments.push(MarkdownSegment::Mermaid {
        source: &markdown[raw.block_start..end],
        diagram: &markdown[raw.diagram_start..end],
    });
    *emitted = end;
}

fn raw_mermaid_indent(line: &str) -> Option<usize> {
    let trimmed = line.trim_start_matches(' ');
    let indent = line.len() - trimmed.len();
    (indent <= 3 && is_raw_mermaid_header(trimmed.trim_end())).then_some(indent)
}

fn is_raw_mermaid_header(header: &str) -> bool {
    matches!(
        header,
        "sequenceDiagram"
            | "stateDiagram-v2"
            | "classDiagram"
            | "erDiagram"
            | "gantt"
            | "timeline"
            | "pie"
            | "xychart-beta"
            | "mindmap"
            | "quadrantChart"
            | "gitGraph"
    ) || raw_flowchart_header(header)
        || header
            .strip_prefix("pie")
            .is_some_and(|suffix| suffix.chars().next().is_some_and(char::is_whitespace))
}

fn raw_flowchart_header(header: &str) -> bool {
    let Some(suffix) = header.strip_prefix("flowchart") else {
        return false;
    };
    suffix.is_empty()
        || (suffix.chars().next().is_some_and(char::is_whitespace)
            && matches!(suffix.trim(), "TD" | "TB" | "BT" | "RL" | "LR"))
}

fn fence_line(line: &str) -> Option<FenceLine<'_>> {
    let marker = line.chars().next()?;
    if !matches!(marker, '`' | '~') {
        return None;
    }
    let marker_len = line
        .chars()
        .take_while(|candidate| *candidate == marker)
        .count();
    (marker_len >= 3).then(|| FenceLine {
        marker,
        marker_len,
        remainder: &line[marker_len..],
    })
}

fn is_closing_fence(line: &str, marker: char, marker_len: usize) -> bool {
    fence_line(line).is_some_and(|fence| {
        fence.marker == marker
            && fence.marker_len >= marker_len
            && fence.remainder.trim().is_empty()
    })
}

fn render_mermaid(diagram: &str, width: usize) -> Result<Vec<HyperlinkLine>> {
    if diagram.trim().is_empty() {
        bail!("empty Mermaid diagram");
    }
    if diagram.len() > MAX_SOURCE_BYTES {
        bail!("Mermaid diagram exceeds {MAX_SOURCE_BYTES} bytes");
    }
    if diagram.contains("%%{") {
        bail!("Mermaid directives are not supported");
    }

    crate::mermaid_runtime::render(diagram, width)
}

#[cfg(test)]
#[path = "mermaid_render_tests.rs"]
mod tests;
