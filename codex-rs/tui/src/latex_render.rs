//! Local LaTeX rendering for display and inline math in assistant Markdown.
//!
//! Rendering is deliberately opportunistic: supported Kitty/Ghostty terminals get image
//! placeholders when `latex` and `dvipng` are available, while every other environment keeps the
//! original Markdown source. Images are cached below `CODEX_HOME` and transmitted as Kitty virtual
//! placements immediately before their history lines are inserted.

use std::collections::HashMap;
use std::collections::HashSet;
use std::env;
use std::fs;
use std::io;
use std::io::Write;
use std::path::Path;
use std::sync::Mutex;
use std::sync::OnceLock;
use std::time::Duration;
use std::time::SystemTime;

use anyhow::Result;
use codex_terminal_detection::TerminalName;
use ratatui::style::Style;
use ratatui::text::Line;
use ratatui::text::Span;
use walkdir::WalkDir;

use crate::terminal_hyperlinks::HyperlinkLine;
use crate::terminal_palette::rgb_color;

mod inline;
mod render;

use render::FormulaLayout;
use render::RenderedFormula;
pub(crate) use render::render_cached_display_png;
#[cfg(test)]
pub(crate) use render::render_cached_display_png_with_metrics;
use render::render_formula;
pub(crate) use render::terminal_cell_pixels;
#[cfg(test)]
use render::*;

const PLACEHOLDER: char = '\u{10eeee}';

// Kitty's normalization-safe row/column diacritics. Formula images are capped well below this
// length, so keeping the first 64 entries avoids carrying the entire 297-codepoint table.
const DIACRITICS: &[char] = &[
    '\u{0305}', '\u{030d}', '\u{030e}', '\u{0310}', '\u{0312}', '\u{033d}', '\u{033e}', '\u{033f}',
    '\u{0346}', '\u{034a}', '\u{034b}', '\u{034c}', '\u{0350}', '\u{0351}', '\u{0352}', '\u{0357}',
    '\u{035b}', '\u{0363}', '\u{0364}', '\u{0365}', '\u{0366}', '\u{0367}', '\u{0368}', '\u{0369}',
    '\u{036a}', '\u{036b}', '\u{036c}', '\u{036d}', '\u{036e}', '\u{036f}', '\u{0483}', '\u{0484}',
    '\u{0485}', '\u{0486}', '\u{0487}', '\u{0592}', '\u{0593}', '\u{0594}', '\u{0595}', '\u{0597}',
    '\u{0598}', '\u{0599}', '\u{059c}', '\u{059d}', '\u{059e}', '\u{059f}', '\u{05a0}', '\u{05a1}',
    '\u{05a8}', '\u{05a9}', '\u{05ab}', '\u{05ac}', '\u{05af}', '\u{05c4}', '\u{0610}', '\u{0611}',
    '\u{0612}', '\u{0613}', '\u{0614}', '\u{0615}', '\u{0616}', '\u{0617}', '\u{0657}', '\u{0658}',
];

#[derive(Debug, Clone, PartialEq, Eq)]
enum MarkdownSegment<'a> {
    Markdown(&'a str),
    DisplayMath { source: &'a str, formula: &'a str },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DisplayDelimiter {
    Dollar,
    Bracket,
}

impl DisplayDelimiter {
    fn opening(line: &str) -> Option<Self> {
        match line {
            "$$" => Some(Self::Dollar),
            r"\[" => Some(Self::Bracket),
            _ => None,
        }
    }

    fn closing(self) -> &'static str {
        match self {
            Self::Dollar => "$$",
            Self::Bracket => r"\]",
        }
    }

    fn same_line_formula(self, line: &str) -> Option<&str> {
        let opening = match self {
            Self::Dollar => "$$",
            Self::Bracket => r"\[",
        };
        let closing = self.closing();
        let formula = line.strip_prefix(opening)?.strip_suffix(closing)?;
        (!formula.is_empty()).then(|| formula.trim())
    }
}

#[derive(Debug, Clone, Copy)]
struct OpenDisplayMath {
    block_start: usize,
    formula_start: usize,
    delimiter: DisplayDelimiter,
}

#[derive(Default)]
struct UploadState {
    owners: HashMap<u32, String>,
    sent: HashSet<u32>,
    pending: Vec<(u32, String)>,
}

static UPLOADS: OnceLock<Mutex<UploadState>> = OnceLock::new();

const CACHE_TTL: Duration = Duration::from_secs(30 * 24 * 60 * 60);

pub(crate) fn expire_cached_pngs(root: &Path) {
    let cutoff = SystemTime::now() - CACHE_TTL;
    for entry in WalkDir::new(root)
        .into_iter()
        .filter_map(|entry| entry.ok())
    {
        let path = entry.path();
        if entry.file_type().is_file()
            && path.extension().is_some_and(|extension| extension == "png")
            && entry
                .metadata()
                .is_ok_and(|metadata| metadata.modified().is_ok_and(|modified| modified < cutoff))
        {
            let _ = fs::remove_file(path);
        }
    }
}

pub(crate) fn render_markdown_with_latex(
    markdown: &str,
    width: Option<usize>,
    feature_enabled: bool,
    render_markdown: impl Fn(&str) -> Vec<HyperlinkLine>,
) -> Vec<HyperlinkLine> {
    if !latex_rendering_enabled(feature_enabled) || !may_contain_latex_math(markdown) {
        return render_markdown(markdown);
    }

    let segments = parse_display_math(markdown);
    let mut lines = Vec::new();
    for segment in segments {
        match segment {
            MarkdownSegment::Markdown(source) => {
                lines.extend(inline::render_markdown_with_inline_latex(
                    source,
                    width,
                    &render_markdown,
                ));
            }
            MarkdownSegment::DisplayMath { source, formula } => {
                let Some(width) = width.filter(|width| *width > 0) else {
                    lines.extend(render_markdown(source));
                    continue;
                };
                match render_formula(formula, width, FormulaLayout::Display) {
                    Ok(rendered) => {
                        for tile in rendered {
                            lines.extend(placeholder_lines(&tile));
                        }
                    }
                    Err(err) => {
                        tracing::debug!(error = %err, "LaTeX display-math rendering unavailable");
                        lines.extend(render_markdown(source));
                    }
                }
            }
        }
    }
    lines
}

pub(crate) fn contains_latex_math(markdown: &str) -> bool {
    if !may_contain_latex_math(markdown) {
        return false;
    }
    parse_display_math(markdown).iter().any(|segment| {
        matches!(segment, MarkdownSegment::DisplayMath { .. })
            || matches!(segment, MarkdownSegment::Markdown(source) if inline::contains_inline_math(source))
    })
}

fn may_contain_latex_math(markdown: &str) -> bool {
    markdown.contains('$') || markdown.contains(r"\[") || markdown.contains(r"\(")
}

pub(crate) fn flush_pending_uploads(writer: &mut impl Write) -> io::Result<()> {
    let uploads = UPLOADS.get_or_init(Default::default);
    let pending = {
        let mut state = uploads
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        std::mem::take(&mut state.pending)
    };
    if pending.is_empty() {
        return Ok(());
    }

    for (index, (image_id, command)) in pending.iter().enumerate() {
        if let Err(err) = writer.write_all(command.as_bytes()) {
            let mut state = uploads
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            state.pending.extend(pending[index..].iter().cloned());
            return Err(err);
        }
        let mut state = uploads
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        state.sent.insert(*image_id);
    }
    writer.flush()
}

fn latex_rendering_enabled(feature_enabled: bool) -> bool {
    kitty_graphics_enabled("CODEX_LATEX_RENDER", feature_enabled)
}

pub(crate) fn latex_rendering_requested(feature_enabled: bool) -> bool {
    rendering_requested("CODEX_LATEX_RENDER", feature_enabled)
}

pub(crate) fn rendering_requested(variable: &str, feature_enabled: bool) -> bool {
    match env::var(variable).as_deref() {
        Ok("0" | "false" | "off" | "no") => false,
        Ok("1" | "true" | "on" | "yes") => true,
        Ok(_) | Err(_) => feature_enabled,
    }
}

pub(crate) fn kitty_graphics_enabled(variable: &str, feature_enabled: bool) -> bool {
    match env::var(variable).as_deref() {
        Ok("0" | "false" | "off" | "no") => false,
        Ok("1" | "true" | "on" | "yes") => true,
        Ok(_) | Err(_) => feature_enabled && kitty_graphics_supported(),
    }
}

pub(crate) fn kitty_graphics_supported() -> bool {
    if env::var_os("TMUX").is_some()
        || env::var_os("TMUX_PANE").is_some()
        || env::var_os("ZELLIJ").is_some()
    {
        return false;
    }
    if env::var_os("KITTY_WINDOW_ID").is_some() {
        return true;
    }

    matches!(
        codex_terminal_detection::terminal_info().name,
        TerminalName::Kitty | TerminalName::Ghostty
    )
}

fn parse_display_math(markdown: &str) -> Vec<MarkdownSegment<'_>> {
    let mut segments = Vec::new();
    let mut cursor = 0usize;
    let mut emitted = 0usize;
    let mut open_math: Option<OpenDisplayMath> = None;
    let mut fence: Option<(char, usize)> = None;

    for line in markdown.split_inclusive('\n') {
        let line_start = cursor;
        cursor += line.len();
        let trimmed = line.trim();

        if open_math.is_none() {
            if let Some((marker, len)) = fence {
                if fence_delimiter(trimmed).is_some_and(|(candidate, candidate_len)| {
                    candidate == marker && candidate_len >= len
                }) {
                    fence = None;
                }
                continue;
            }
            if let Some(delimiter) = fence_delimiter(trimmed) {
                fence = Some(delimiter);
                continue;
            }
        }

        if fence.is_some() {
            continue;
        }

        if open_math.is_none()
            && let Some(formula) = [DisplayDelimiter::Dollar, DisplayDelimiter::Bracket]
                .into_iter()
                .find_map(|delimiter| delimiter.same_line_formula(trimmed))
        {
            if emitted < line_start {
                segments.push(MarkdownSegment::Markdown(&markdown[emitted..line_start]));
            }
            segments.push(MarkdownSegment::DisplayMath {
                source: &markdown[line_start..cursor],
                formula,
            });
            emitted = cursor;
            continue;
        }

        if let Some(open) = open_math {
            if trimmed != open.delimiter.closing() {
                continue;
            }
            open_math = None;
            if emitted < open.block_start {
                segments.push(MarkdownSegment::Markdown(
                    &markdown[emitted..open.block_start],
                ));
            }
            let source = &markdown[open.block_start..cursor];
            let formula_end = line_start;
            let formula = markdown[open.formula_start..formula_end].trim();
            segments.push(MarkdownSegment::DisplayMath { source, formula });
            emitted = cursor;
        } else if let Some(delimiter) = DisplayDelimiter::opening(trimmed) {
            open_math = Some(OpenDisplayMath {
                block_start: line_start,
                formula_start: cursor,
                delimiter,
            });
        }
    }

    if let Some(open) = open_math {
        if emitted < open.block_start {
            segments.push(MarkdownSegment::Markdown(
                &markdown[emitted..open.block_start],
            ));
        }
        segments.push(MarkdownSegment::Markdown(&markdown[open.block_start..]));
    } else if emitted < markdown.len() {
        segments.push(MarkdownSegment::Markdown(&markdown[emitted..]));
    }

    if segments.is_empty() {
        segments.push(MarkdownSegment::Markdown(markdown));
    }
    segments
}

fn fence_delimiter(trimmed: &str) -> Option<(char, usize)> {
    let marker = trimmed.chars().next()?;
    if !matches!(marker, '`' | '~') {
        return None;
    }
    let len = trimmed
        .chars()
        .take_while(|candidate| *candidate == marker)
        .count();
    (len >= 3).then_some((marker, len))
}

fn placeholder_lines(rendered: &RenderedFormula) -> Vec<HyperlinkLine> {
    let (red, green, blue) = image_id_rgb(rendered.image_id);
    let style = Style::default().fg(rgb_color((red, green, blue)));
    (0..rendered.rows)
        .map(|row| {
            let mut placeholders = String::with_capacity(usize::from(rendered.columns) * 4);
            for column in 0..rendered.columns {
                placeholders.push(PLACEHOLDER);
                if column == 0 {
                    placeholders.push(DIACRITICS[usize::from(row)]);
                }
            }
            HyperlinkLine::new(Line::from(Span::styled(placeholders, style)))
        })
        .collect()
}

/// Plan every inline upload, splice the already-assigned image IDs into the final lines, and only
/// then publish the Kitty commands. This prevents a fallback from leaving an unanchored image in
/// the terminal when one formula cannot be placed.
fn queue_inline_uploads_after<T>(
    replacements: &mut [inline::InlineReplacement],
    finish: impl FnOnce(&[inline::InlineReplacement]) -> Option<T>,
) -> std::result::Result<Option<T>, (usize, anyhow::Error)> {
    let uploads = UPLOADS.get_or_init(Default::default);
    let mut state = uploads
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let mut owners = state.owners.clone();
    let mut pending = Vec::new();

    for (index, replacement) in replacements.iter_mut().enumerate() {
        let rendered = &mut replacement.rendered;
        let Some(path) = rendered.upload_path.as_deref() else {
            continue;
        };
        let owner = path.to_string_lossy().into_owned();
        let image_id = available_image_id(&owners, &owner, rendered.image_id);
        if owners.get(&image_id) != Some(&owner) {
            let command = crate::pets::kitty_transmit_png_virtual_with_id(
                path,
                rendered.columns,
                rendered.rows,
                image_id,
            )
            .map_err(|err| (index, err))?;
            owners.insert(image_id, owner);
            pending.push((image_id, command));
        }
        rendered.image_id = image_id;
    }

    let Some(output) = finish(replacements) else {
        return Ok(None);
    };
    state.owners = owners;
    state.pending.extend(pending);
    Ok(Some(output))
}

fn queue_upload(path: &Path, preferred_image_id: u32, columns: u16, rows: u16) -> Result<u32> {
    let uploads = UPLOADS.get_or_init(Default::default);
    let mut state = uploads
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let owner = path.to_string_lossy().into_owned();
    let image_id = available_image_id(&state.owners, &owner, preferred_image_id);
    if state.owners.get(&image_id) == Some(&owner) {
        return Ok(image_id);
    }
    let command = crate::pets::kitty_transmit_png_virtual_with_id(path, columns, rows, image_id)?;
    state.owners.insert(image_id, owner);
    state.pending.push((image_id, command));
    Ok(image_id)
}

fn available_image_id(owners: &HashMap<u32, String>, owner: &str, preferred_image_id: u32) -> u32 {
    let mut image_id = preferred_image_id.clamp(1, 0x00ff_ffff);
    loop {
        match owners.get(&image_id) {
            Some(existing) if existing == owner => return image_id,
            Some(_) => {
                image_id = if image_id == 0x00ff_ffff {
                    1
                } else {
                    image_id + 1
                };
            }
            None => return image_id,
        }
    }
}

fn image_id_rgb(image_id: u32) -> (u8, u8, u8) {
    (
        ((image_id >> 16) & 0xff) as u8,
        ((image_id >> 8) & 0xff) as u8,
        (image_id & 0xff) as u8,
    )
}

#[cfg(test)]
#[path = "latex_render_tests.rs"]
mod tests;
