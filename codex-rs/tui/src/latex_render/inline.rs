//! Source-aware inline-LaTeX rewriting and Kitty placeholder insertion.

use std::collections::HashSet;
use std::ops::Range;

use pulldown_cmark::Event;
use pulldown_cmark::Options;
use pulldown_cmark::Parser;
use pulldown_cmark::Tag;
use pulldown_cmark::TagEnd;
use ratatui::style::Style;
use ratatui::text::Line;
use ratatui::text::Span;

use super::DIACRITICS;
use super::FormulaLayout;
use super::PLACEHOLDER;
use super::RenderedFormula;
use super::image_id_rgb;
use super::queue_inline_uploads_after;
use super::render_formula;
use crate::terminal_hyperlinks::HyperlinkLine;
use crate::terminal_palette::rgb_color;
use crate::width::char_width;
use crate::width::display_width;

const SENTINEL_START: u32 = 0xe000;
const SENTINEL_END: u32 = 0xf8ff;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct InlineMathToken<'a> {
    pub(super) source: Range<usize>,
    pub(super) formula: &'a str,
}

#[derive(Debug)]
pub(super) struct InlineReplacement {
    pub(super) source: Range<usize>,
    pub(super) token: String,
    pub(super) rendered: RenderedFormula,
}

#[derive(Debug)]
struct LocatedReplacement<'a> {
    range: Range<usize>,
    rendered: &'a RenderedFormula,
}

pub(super) fn contains_inline_math(markdown: &str) -> bool {
    !parse_inline_math(markdown).is_empty()
}

pub(super) fn render_markdown_with_inline_latex(
    markdown: &str,
    width: Option<usize>,
    render_markdown: &impl Fn(&str) -> Vec<HyperlinkLine>,
) -> Vec<HyperlinkLine> {
    let Some(width) = width.filter(|width| *width > 0) else {
        return render_markdown(markdown);
    };
    let tokens = parse_inline_math(markdown);
    if tokens.is_empty() {
        return render_markdown(markdown);
    }

    let mut used_sentinels = HashSet::new();
    let mut replacements = Vec::new();
    for token in tokens {
        let rendered = match render_formula(token.formula, width, FormulaLayout::Inline) {
            Ok(mut rendered) if rendered.len() == 1 => rendered.pop(),
            Ok(_) => None,
            Err(err) => {
                tracing::debug!(error = %err, "inline LaTeX rendering unavailable");
                None
            }
        };
        let Some(rendered) = rendered else {
            continue;
        };
        let Some(sentinel) = unused_sentinel(markdown, &used_sentinels) else {
            continue;
        };
        used_sentinels.insert(sentinel);
        let sentinel_token = sentinel_token(sentinel, usize::from(rendered.columns));
        if !replacement_fits(markdown, &token.source, &sentinel_token, width) {
            continue;
        }
        replacements.push(InlineReplacement {
            source: token.source,
            token: sentinel_token,
            rendered,
        });
    }
    if replacements.is_empty() {
        return render_markdown(markdown);
    }

    loop {
        let rewritten = rewrite_with_replacements(markdown, &replacements);
        let lines = render_markdown(&rewritten);
        let invalid = invalid_replacement_indices(&lines, &replacements);
        if !invalid.is_empty() {
            remove_replacements(&mut replacements, &invalid);
            if replacements.is_empty() {
                return render_markdown(markdown);
            }
            continue;
        }

        match queue_inline_uploads_after(&mut replacements, |replacements| {
            replace_inline_sentinels(lines, replacements)
        }) {
            Ok(Some(lines)) => return lines,
            Ok(None) => return render_markdown(markdown),
            Err((index, err)) => {
                tracing::debug!(error = %err, "inline LaTeX upload unavailable");
                remove_replacements(&mut replacements, &[index]);
                if replacements.is_empty() {
                    return render_markdown(markdown);
                }
            }
        }
    }
}

pub(super) fn rewrite_with_replacements(
    markdown: &str,
    replacements: &[InlineReplacement],
) -> String {
    let mut rewritten = String::with_capacity(markdown.len());
    let mut cursor = 0usize;
    for replacement in replacements {
        rewritten.push_str(&markdown[cursor..replacement.source.start]);
        rewritten.push_str(&replacement.token);
        cursor = replacement.source.end;
    }
    rewritten.push_str(&markdown[cursor..]);
    rewritten
}

pub(super) fn invalid_replacement_indices(
    lines: &[HyperlinkLine],
    replacements: &[InlineReplacement],
) -> Vec<usize> {
    let flattened = lines
        .iter()
        .map(|line| {
            line.line
                .spans
                .iter()
                .map(|span| span.content.as_ref())
                .collect::<String>()
        })
        .collect::<Vec<_>>();
    replacements
        .iter()
        .enumerate()
        .filter_map(|(index, replacement)| {
            let count = flattened
                .iter()
                .map(|line| line.match_indices(&replacement.token).count())
                .sum::<usize>();
            (count != 1).then_some(index)
        })
        .collect()
}

pub(super) fn remove_replacements(replacements: &mut Vec<InlineReplacement>, indices: &[usize]) {
    for index in indices.iter().rev() {
        replacements.remove(*index);
    }
}

pub(super) fn parse_inline_math(markdown: &str) -> Vec<InlineMathToken<'_>> {
    if !markdown.contains('$') && !markdown.contains(r"\(") {
        return Vec::new();
    }

    let (forbidden, recognized) = markdown_ranges(markdown);
    let mut tokens = Vec::new();
    let mut cursor = 0usize;
    while cursor < markdown.len() {
        if markdown[cursor..].starts_with("$$") {
            cursor += 2;
            continue;
        }
        if markdown[cursor..].starts_with('$')
            && !is_escaped(markdown, cursor)
            && !markdown[..cursor].ends_with('$')
        {
            if let Some(close) = find_dollar_close(markdown, cursor + 1) {
                let source = cursor..close + 1;
                let formula = &markdown[cursor + 1..close];
                if valid_dollar_formula(formula)
                    && !intersects_any(&source, &forbidden)
                    && covered_by_any(&source, &recognized)
                {
                    tokens.push(InlineMathToken { source, formula });
                    cursor = close + 1;
                    continue;
                }
            }
        } else if markdown[cursor..].starts_with(r"\(")
            && !is_escaped(markdown, cursor)
            && let Some(close) = find_parenthesis_close(markdown, cursor + 2)
        {
            let source = cursor..close + 2;
            let formula = &markdown[cursor + 2..close];
            if valid_parenthesized_formula(formula)
                && !intersects_any(&source, &forbidden)
                && covered_by_any(&source, &recognized)
            {
                tokens.push(InlineMathToken { source, formula });
                cursor = close + 2;
                continue;
            }
        }
        cursor += markdown[cursor..].chars().next().map_or(1, char::len_utf8);
    }
    tokens
}

fn markdown_ranges(markdown: &str) -> (Vec<Range<usize>>, Vec<Range<usize>>) {
    let mut options = Options::empty();
    options.insert(Options::ENABLE_STRIKETHROUGH);
    options.insert(Options::ENABLE_TABLES);
    let mut ranges = Vec::new();
    let mut recognized = Vec::new();
    let mut code_block_start = None;
    let mut link_or_image_starts = Vec::new();

    for (event, range) in Parser::new_ext(markdown, options).into_offset_iter() {
        match event {
            Event::Start(Tag::Paragraph | Tag::Heading { .. } | Tag::TableCell) => {
                recognized.push(range);
            }
            Event::Text(_) => recognized.push(range),
            Event::Start(Tag::CodeBlock(_)) => code_block_start = Some(range.start),
            Event::End(TagEnd::CodeBlock) => {
                if let Some(start) = code_block_start.take() {
                    ranges.push(start..range.end);
                }
            }
            Event::Start(Tag::Link { .. } | Tag::Image { .. }) => {
                link_or_image_starts.push(range.start);
            }
            Event::End(TagEnd::Link | TagEnd::Image) => {
                if let Some(start) = link_or_image_starts.pop() {
                    ranges.push(start..range.end);
                }
            }
            Event::Code(_) | Event::Html(_) | Event::InlineHtml(_) => ranges.push(range),
            _ => {}
        }
    }
    if let Some(start) = code_block_start {
        ranges.push(start..markdown.len());
    }
    for start in link_or_image_starts {
        ranges.push(start..markdown.len());
    }
    (merge_ranges(ranges), merge_ranges(recognized))
}

fn merge_ranges(mut ranges: Vec<Range<usize>>) -> Vec<Range<usize>> {
    ranges.sort_by_key(|range| range.start);
    let mut merged: Vec<Range<usize>> = Vec::new();
    for range in ranges {
        if let Some(previous) = merged.last_mut()
            && range.start <= previous.end
        {
            previous.end = previous.end.max(range.end);
        } else {
            merged.push(range);
        }
    }
    merged
}

fn find_dollar_close(markdown: &str, mut cursor: usize) -> Option<usize> {
    while cursor < markdown.len() {
        if markdown[cursor..].starts_with("$$") {
            return None;
        }
        if markdown[cursor..].starts_with('$')
            && !is_escaped(markdown, cursor)
            && !markdown[..cursor].ends_with('$')
            && !markdown[cursor + 1..].starts_with('$')
        {
            return Some(cursor);
        }
        cursor += markdown[cursor..].chars().next()?.len_utf8();
    }
    None
}

fn find_parenthesis_close(markdown: &str, mut cursor: usize) -> Option<usize> {
    while cursor < markdown.len() {
        if markdown[cursor..].starts_with(r"\)") && !is_escaped(markdown, cursor) {
            return Some(cursor);
        }
        cursor += markdown[cursor..].chars().next()?.len_utf8();
    }
    None
}

fn valid_dollar_formula(formula: &str) -> bool {
    !formula.is_empty()
        && !formula.contains(['\r', '\n'])
        && formula.chars().next().is_some_and(|ch| !ch.is_whitespace())
        && formula
            .chars()
            .next_back()
            .is_some_and(|ch| !ch.is_whitespace())
}

fn valid_parenthesized_formula(formula: &str) -> bool {
    !formula.trim().is_empty() && !formula.contains(['\r', '\n'])
}

fn is_escaped(source: &str, byte: usize) -> bool {
    source.as_bytes()[..byte]
        .iter()
        .rev()
        .take_while(|candidate| **candidate == b'\\')
        .count()
        % 2
        == 1
}

fn intersects_any(candidate: &Range<usize>, forbidden: &[Range<usize>]) -> bool {
    forbidden
        .iter()
        .any(|range| candidate.start < range.end && range.start < candidate.end)
}

fn covered_by_any(candidate: &Range<usize>, recognized: &[Range<usize>]) -> bool {
    recognized
        .iter()
        .any(|range| range.start <= candidate.start && candidate.end <= range.end)
}

fn unused_sentinel(markdown: &str, used: &HashSet<char>) -> Option<char> {
    (SENTINEL_START..=SENTINEL_END)
        .filter_map(char::from_u32)
        .find(|candidate| {
            char_width(*candidate) == 1
                && !used.contains(candidate)
                && !markdown.contains(*candidate)
        })
}

fn sentinel_token(sentinel: char, columns: usize) -> String {
    if columns >= 5 {
        let mut token = String::from("x://");
        token.extend(std::iter::repeat_n(sentinel, columns - 4));
        token
    } else {
        sentinel.to_string().repeat(columns)
    }
}

pub(super) fn replacement_fits(
    markdown: &str,
    source: &Range<usize>,
    replacement: &str,
    width: usize,
) -> bool {
    if display_width(replacement) > width {
        return false;
    }
    let run_start = markdown[..source.start]
        .rfind(|ch: char| ch.is_ascii_whitespace())
        .map_or(0, |start| {
            start + markdown[start..].chars().next().map_or(0, char::len_utf8)
        });
    let run_end = markdown[source.end..]
        .find(|ch: char| ch.is_ascii_whitespace())
        .map_or(markdown.len(), |end| source.end + end);
    let run = &markdown[run_start..run_end];
    if run.contains("://") {
        return false;
    }
    display_width(&markdown[run_start..source.start])
        .saturating_add(display_width(replacement))
        .saturating_add(display_width(&markdown[source.end..run_end]))
        <= width
}

pub(super) fn replace_inline_sentinels(
    lines: Vec<HyperlinkLine>,
    replacements: &[InlineReplacement],
) -> Option<Vec<HyperlinkLine>> {
    let flattened = lines
        .iter()
        .map(|line| {
            line.line
                .spans
                .iter()
                .map(|span| span.content.as_ref())
                .collect::<String>()
        })
        .collect::<Vec<_>>();
    let mut by_line = (0..lines.len())
        .map(|_| Vec::<LocatedReplacement<'_>>::new())
        .collect::<Vec<_>>();

    for replacement in replacements {
        let mut found = None;
        for (line_index, text) in flattened.iter().enumerate() {
            for (start, _) in text.match_indices(&replacement.token) {
                if found.is_some() {
                    return None;
                }
                found = Some((line_index, start));
            }
        }
        let (line_index, start) = found?;
        by_line[line_index].push(LocatedReplacement {
            range: start..start + replacement.token.len(),
            rendered: &replacement.rendered,
        });
    }

    let mut output = Vec::with_capacity(lines.len());
    for (line_index, (mut line, mut located)) in lines.into_iter().zip(by_line).enumerate() {
        if located.is_empty() {
            output.push(line);
            continue;
        }
        let multiline_count = located
            .iter()
            .filter(|replacement| replacement.rendered.rows > 1)
            .count();
        if multiline_count > 1
            || located
                .iter()
                .any(|replacement| replacement.rendered.rows > DIACRITICS.len() as u16)
        {
            return None;
        }
        located.sort_by_key(|replacement| replacement.range.start);
        let original_width = line.width();
        let mut spans = Vec::with_capacity(line.line.spans.len() + located.len());
        let mut global = 0usize;
        let mut replacement_index = 0usize;

        for span in &line.line.spans {
            let text = span.content.as_ref();
            let span_end = global + text.len();
            let mut cursor = global;
            while cursor < span_end {
                let Some(replacement) = located.get(replacement_index) else {
                    push_text_span(&mut spans, &text[cursor - global..], span.style);
                    break;
                };
                if replacement.range.start >= span_end {
                    push_text_span(&mut spans, &text[cursor - global..], span.style);
                    break;
                }
                if cursor < replacement.range.start {
                    let end = replacement.range.start.min(span_end);
                    push_text_span(&mut spans, &text[cursor - global..end - global], span.style);
                    cursor = end;
                    continue;
                }
                if cursor == replacement.range.start {
                    spans.push(inline_placeholder_span(replacement.rendered, span.style));
                }
                cursor = replacement.range.end.min(span_end);
                if cursor == replacement.range.end {
                    replacement_index += 1;
                }
            }
            global = span_end;
        }

        line.line = Line {
            style: line.line.style,
            alignment: line.line.alignment,
            spans,
        };
        if line.width() != original_width {
            return None;
        }
        let Some(replacement) = (multiline_count == 1)
            .then(|| {
                located
                    .iter()
                    .find(|replacement| replacement.rendered.rows > 1)
            })
            .flatten()
        else {
            output.push(line);
            continue;
        };
        if !line.hyperlinks.is_empty() {
            return None;
        }
        let prefix_width = display_width(&flattened[line_index][..replacement.range.start]);
        output.push(line);
        for row in 1..replacement.rendered.rows {
            output.push(inline_continuation_line(
                replacement.rendered,
                prefix_width,
                row as usize,
            ));
        }
    }
    Some(output)
}

pub(super) fn inline_placeholder_span(rendered: &RenderedFormula, base: Style) -> Span<'static> {
    inline_placeholder_span_for_row(rendered, base, 0)
}

fn inline_placeholder_span_for_row(
    rendered: &RenderedFormula,
    base: Style,
    row: usize,
) -> Span<'static> {
    let (red, green, blue) = image_id_rgb(rendered.image_id);
    let image_style = Style::default().fg(rgb_color((red, green, blue)));
    let mut placeholders = String::with_capacity(usize::from(rendered.columns) * 4);
    for column in 0..rendered.columns {
        placeholders.push(PLACEHOLDER);
        if column == 0 {
            placeholders.push(DIACRITICS[row]);
        }
    }
    Span::styled(placeholders, base.patch(image_style))
}

fn inline_continuation_line(
    rendered: &RenderedFormula,
    prefix_width: usize,
    row: usize,
) -> HyperlinkLine {
    let mut line = Line::default();
    if prefix_width > 0 {
        line.push_span(Span::raw(" ".repeat(prefix_width)));
    }
    line.push_span(inline_placeholder_span_for_row(
        rendered,
        Style::default(),
        row,
    ));
    HyperlinkLine::new(line)
}

fn push_text_span(spans: &mut Vec<Span<'static>>, text: &str, style: Style) {
    if !text.is_empty() {
        spans.push(Span::styled(text.to_owned(), style));
    }
}
