//! LaTeX compilation, PNG post-processing, caching, and display tiling.

use std::fs;
use std::path::Path;
use std::path::PathBuf;
use std::process::Command;
use std::process::Stdio;
use std::time::Duration;
use std::time::Instant;

use anyhow::Context;
use anyhow::Result;
use anyhow::bail;
use image::ImageFormat;
use image::RgbaImage;
use image::imageops::FilterType;
use sha2::Digest;
use sha2::Sha256;

use super::DIACRITICS;
use super::placeholder_lines;
use super::queue_upload;
use crate::terminal_hyperlinks::HyperlinkLine;

const CACHE_VERSION: &str = "v7";
const RENDER_TIMEOUT: Duration = Duration::from_secs(8);
const DEFAULT_CELL_WIDTH_PX: u16 = 10;
const DEFAULT_CELL_HEIGHT_PX: u16 = 20;
const DISPLAY_SCALE_NUMERATOR: u32 = 1;
const DISPLAY_SCALE_DENOMINATOR: u32 = 1;
const INLINE_HEIGHT_NUMERATOR: u32 = 19;
const INLINE_HEIGHT_DENOMINATOR: u32 = 20;
const INLINE_SHORT_HEIGHT_NUMERATOR: u32 = 14;
const INLINE_SHORT_ASPECT_LIMIT: u32 = 2;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum FormulaLayout {
    Display,
    Inline,
}

impl FormulaLayout {
    fn cache_tag(self) -> &'static str {
        match self {
            Self::Display => "display-full-size-tiles",
            Self::Inline => "inline-95-percent-row",
        }
    }
}

#[derive(Debug, Clone)]
pub(super) struct RenderedFormula {
    pub(super) image_id: u32,
    pub(super) columns: u16,
    pub(super) rows: u16,
    pub(super) upload_path: Option<PathBuf>,
}

pub(super) fn render_formula(
    formula: &str,
    available_columns: usize,
    layout: FormulaLayout,
) -> Result<Vec<RenderedFormula>> {
    if formula.is_empty() {
        bail!("empty LaTeX formula");
    }

    let latex = resolve_executable("latex").context("latex executable not found")?;
    let dvipng = resolve_executable("dvipng").context("dvipng executable not found")?;
    let (cell_width_px, cell_height_px) = terminal_cell_pixels();
    let foreground = crate::terminal_palette::default_fg().unwrap_or((220, 220, 220));
    let cache_key = cache_key(
        formula,
        layout,
        available_columns,
        cell_width_px,
        cell_height_px,
        foreground,
    );
    let image_id = image_id_from_key(&cache_key);
    let cache_dir = latex_cache_dir()?;
    fs::create_dir_all(&cache_dir)
        .with_context(|| format!("create LaTeX cache {}", cache_dir.display()))?;
    let path = cache_dir.join(format!("{cache_key}.png"));

    if !path.is_file() {
        compile_formula(
            formula,
            layout,
            &latex,
            &dvipng,
            &path,
            available_columns,
            cell_width_px,
            cell_height_px,
            foreground,
        )?;
    }

    let image = image::open(&path).with_context(|| format!("read {}", path.display()))?;
    match layout {
        FormulaLayout::Display => tile_display_formula(
            &image.to_rgba8(),
            &path,
            &cache_key,
            available_columns,
            cell_width_px,
            cell_height_px,
        ),
        FormulaLayout::Inline => {
            let columns = div_ceil_u32(image.width(), u32::from(cell_width_px))
                .clamp(1, available_columns.min(usize::from(u16::MAX)) as u32)
                as u16;
            let rows = div_ceil_u32(image.height(), u32::from(cell_height_px))
                .clamp(1, u32::from(u16::MAX)) as u16;
            Ok(vec![RenderedFormula {
                image_id,
                columns,
                rows,
                upload_path: Some(path),
            }])
        }
    }
}

pub(crate) fn render_cached_display_png(
    path: &Path,
    cache_key: &str,
    available_columns: usize,
    display_width_scale_halves: u32,
) -> Result<Vec<HyperlinkLine>> {
    let (cell_width_px, cell_height_px) = terminal_cell_pixels();
    render_cached_display_png_with_metrics(
        path,
        cache_key,
        available_columns,
        cell_width_px,
        cell_height_px,
        display_width_scale_halves,
    )
}

pub(crate) fn render_cached_display_png_with_metrics(
    path: &Path,
    cache_key: &str,
    available_columns: usize,
    cell_width_px: u16,
    cell_height_px: u16,
    display_width_scale_halves: u32,
) -> Result<Vec<HyperlinkLine>> {
    let image = image::open(path)
        .with_context(|| format!("read terminal image {}", path.display()))?
        .to_rgba8();
    let available_width_px = u32::try_from(available_columns)
        .unwrap_or(u32::MAX)
        .saturating_mul(u32::from(cell_width_px));
    let intrinsic_columns = div_ceil_u32(
        image.width(),
        u32::from(DEFAULT_CELL_WIDTH_PX).saturating_mul(crate::mermaid_runtime::RENDER_SCALE),
    );
    let display_width_scale_halves = if image.height().saturating_mul(2) >= image.width() {
        display_width_scale_halves.max(2)
    } else {
        2
    };
    let target_width_px = available_width_px
        .min(
            intrinsic_columns
                .saturating_mul(u32::from(cell_width_px))
                .saturating_mul(display_width_scale_halves)
                .div_ceil(2),
        )
        .min(image.width());
    let image = if target_width_px > 0 && image.width() > target_width_px {
        let height = (u64::from(image.height()) * u64::from(target_width_px))
            .div_ceil(u64::from(image.width()))
            .max(1) as u32;
        image::imageops::resize(&image, target_width_px, height, FilterType::Lanczos3)
    } else {
        image
    };
    let tile_key = format!(
        "{cache_key}-display-fit-v6-s{display_width_scale_halves}-cw{cell_width_px}-ch{cell_height_px}"
    );
    let rendered = tile_display_formula(
        &image,
        path,
        &tile_key,
        available_columns,
        cell_width_px,
        cell_height_px,
    )?;
    Ok(rendered
        .iter()
        .flat_map(placeholder_lines)
        .collect::<Vec<_>>())
}

#[allow(clippy::too_many_arguments)]
pub(super) fn compile_formula(
    formula: &str,
    layout: FormulaLayout,
    latex: &Path,
    dvipng: &Path,
    cache_path: &Path,
    available_columns: usize,
    cell_width_px: u16,
    cell_height_px: u16,
    foreground: (u8, u8, u8),
) -> Result<()> {
    let temp = tempfile::tempdir().context("create LaTeX render directory")?;
    let tex_path = temp.path().join("formula.tex");
    fs::write(&tex_path, latex_document(formula, layout)).context("write formula.tex")?;

    let mut latex_command = Command::new(latex);
    latex_command
        .current_dir(temp.path())
        .env("openin_any", "p")
        .env("openout_any", "p")
        .args([
            "-interaction=nonstopmode",
            "-halt-on-error",
            "-no-shell-escape",
            "formula.tex",
        ]);
    run_with_timeout(&mut latex_command).context("latex failed")?;

    let raw_path = temp.path().join("formula.png");
    let foreground = format!(
        "rgb {:.4} {:.4} {:.4}",
        f32::from(foreground.0) / 255.0,
        f32::from(foreground.1) / 255.0,
        f32::from(foreground.2) / 255.0
    );
    let mut dvipng_command = Command::new(dvipng);
    dvipng_command.current_dir(temp.path()).args([
        "-D",
        "240",
        "-T",
        "tight",
        "-bg",
        "Transparent",
        "-fg",
        &foreground,
        "-o",
        raw_path.to_string_lossy().as_ref(),
        "formula.dvi",
    ]);
    run_with_timeout(&mut dvipng_command).context("dvipng failed")?;

    post_process_png(
        &raw_path,
        cache_path,
        layout,
        available_columns,
        cell_width_px,
        cell_height_px,
    )
}

fn run_with_timeout(command: &mut Command) -> Result<()> {
    let mut child = command
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .context("spawn renderer")?;
    let started = Instant::now();
    loop {
        if let Some(status) = child.try_wait().context("wait for renderer")? {
            if status.success() {
                return Ok(());
            }
            bail!("renderer exited with {status}");
        }
        if started.elapsed() >= RENDER_TIMEOUT {
            let _ = child.kill();
            let _ = child.wait();
            bail!("renderer timed out after {}s", RENDER_TIMEOUT.as_secs());
        }
        std::thread::sleep(Duration::from_millis(10));
    }
}

pub(super) fn post_process_png(
    source: &Path,
    destination: &Path,
    layout: FormulaLayout,
    available_columns: usize,
    cell_width_px: u16,
    cell_height_px: u16,
) -> Result<()> {
    let image = image::open(source)
        .with_context(|| format!("read rendered formula {}", source.display()))?
        .to_rgba8();
    let padded = match layout {
        FormulaLayout::Display => {
            let content = image::imageops::resize(
                &image,
                image
                    .width()
                    .saturating_mul(DISPLAY_SCALE_NUMERATOR)
                    .div_ceil(DISPLAY_SCALE_DENOMINATOR)
                    .max(1),
                image
                    .height()
                    .saturating_mul(DISPLAY_SCALE_NUMERATOR)
                    .div_ceil(DISPLAY_SCALE_DENOMINATOR)
                    .max(1),
                FilterType::Lanczos3,
            );
            let horizontal_padding = u32::from(cell_width_px) / 2;
            let vertical_padding = u32::from(cell_height_px) / 5;
            let mut padded = RgbaImage::new(
                content.width() + horizontal_padding * 2,
                content.height() + vertical_padding * 2,
            );
            image::imageops::overlay(
                &mut padded,
                &content,
                i64::from(horizontal_padding),
                i64::from(vertical_padding),
            );
            padded
        }
        FormulaLayout::Inline => {
            let horizontal_padding = u32::from(cell_width_px) / 4;
            let max_width = available_columns
                .saturating_mul(usize::from(cell_width_px))
                .max(1) as u32;
            let content_max_width = max_width.saturating_sub(horizontal_padding * 2).max(1);
            let height_numerator = (image.width()
                <= image.height().saturating_mul(INLINE_SHORT_ASPECT_LIMIT))
            .then_some(INLINE_SHORT_HEIGHT_NUMERATOR)
            .unwrap_or(INLINE_HEIGHT_NUMERATOR);
            let content_max_height = u32::from(cell_height_px).saturating_mul(height_numerator)
                / INLINE_HEIGHT_DENOMINATOR;
            let width_scale = f64::from(content_max_width) / f64::from(image.width().max(1));
            let height_scale =
                f64::from(content_max_height.max(1)) / f64::from(image.height().max(1));
            // Inline formulas share a text line with terminal glyphs.  Let the
            // one-cell height target win even for short/tall glyphs; keeping a
            // 1.0 minimum makes symbols such as `\\gamma^\\mu` render at full
            // 12pt size and spill into a second placeholder row.
            let scale = width_scale.min(height_scale);
            let content_width = (f64::from(image.width()) * scale).round().max(1.0) as u32;
            let content_height = (f64::from(image.height()) * scale).round().max(1.0) as u32;
            let content = image::imageops::resize(
                &image,
                content_width,
                content_height,
                FilterType::Lanczos3,
            );
            let unrounded_width = content.width() + horizontal_padding * 2;
            let padded_width = div_ceil_u32(unrounded_width, u32::from(cell_width_px))
                .max(1)
                .saturating_mul(u32::from(cell_width_px));
            let padded_height = div_ceil_u32(content.height(), u32::from(cell_height_px))
                .max(1)
                .saturating_mul(u32::from(cell_height_px));
            let mut padded = RgbaImage::new(padded_width, padded_height);
            let top = padded.height().saturating_sub(content.height()) / 2;
            image::imageops::overlay(
                &mut padded,
                &content,
                i64::from(horizontal_padding),
                i64::from(top),
            );
            padded
        }
    };
    padded
        .save_with_format(destination, ImageFormat::Png)
        .with_context(|| format!("write LaTeX cache {}", destination.display()))
}

pub(super) fn latex_document(formula: &str, layout: FormulaLayout) -> String {
    let body = match layout {
        FormulaLayout::Display => format!("\\[\n{formula}\n\\]"),
        FormulaLayout::Inline => format!("\\noindent\\({formula}\\)"),
    };
    format!(
        "\\documentclass[12pt]{{article}}\n\
         \\usepackage{{amsmath,amssymb}}\n\
         \\pagestyle{{empty}}\n\
         \\begin{{document}}\n\
         {body}\n\
         \\end{{document}}\n"
    )
}

pub(super) fn tile_display_formula(
    image: &RgbaImage,
    cache_path: &Path,
    cache_key: &str,
    available_columns: usize,
    cell_width_px: u16,
    cell_height_px: u16,
) -> Result<Vec<RenderedFormula>> {
    let tile_columns = available_columns
        .clamp(1, usize::from(u16::MAX))
        .try_into()
        .unwrap_or(u16::MAX);
    let max_tile_width = u32::from(tile_columns).saturating_mul(u32::from(cell_width_px));
    let max_tile_height = (DIACRITICS.len() as u32).saturating_mul(u32::from(cell_height_px));
    let cache_dir = cache_path
        .parent()
        .context("LaTeX cache path has no parent")?;
    let mut rendered = Vec::new();

    for (tile_x, x) in (0..image.width())
        .step_by(max_tile_width.max(1) as usize)
        .enumerate()
    {
        for (tile_y, y) in (0..image.height())
            .step_by(max_tile_height.max(1) as usize)
            .enumerate()
        {
            let width = image.width().saturating_sub(x).min(max_tile_width);
            let height = image.height().saturating_sub(y).min(max_tile_height);
            let columns = div_ceil_u32(width, u32::from(cell_width_px)).max(1) as u16;
            let rows = div_ceil_u32(height, u32::from(cell_height_px)).max(1) as u16;
            let tile_key = format!("{cache_key}-c{tile_columns}-x{tile_x}-y{tile_y}");
            let preferred_image_id = image_id_from_key(&format!("{:x}", Sha256::digest(&tile_key)));
            let tile_path = cache_dir.join(format!("{tile_key}.png"));
            if !tile_path.is_file() {
                let crop = image::imageops::crop_imm(image, x, y, width, height).to_image();
                let mut padded = RgbaImage::new(
                    u32::from(columns).saturating_mul(u32::from(cell_width_px)),
                    u32::from(rows).saturating_mul(u32::from(cell_height_px)),
                );
                image::imageops::overlay(&mut padded, &crop, 0, 0);
                padded
                    .save_with_format(&tile_path, ImageFormat::Png)
                    .with_context(|| format!("write LaTeX tile {}", tile_path.display()))?;
            }
            let tile_image_id = queue_upload(&tile_path, preferred_image_id, columns, rows)?;
            rendered.push(RenderedFormula {
                image_id: tile_image_id,
                columns,
                rows,
                upload_path: None,
            });
        }
    }
    if rendered.is_empty() {
        bail!("rendered formula image is empty");
    }
    Ok(rendered)
}

pub(super) fn resolve_executable(name: &str) -> Option<PathBuf> {
    let path = PathBuf::from(name);
    if Command::new(&path)
        .arg("--version")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok_and(|status| status.success())
    {
        return Some(path);
    }

    #[cfg(target_os = "macos")]
    {
        let path = PathBuf::from("/Library/TeX/texbin").join(name);
        if path.is_file() {
            return Some(path);
        }
    }
    None
}

fn latex_cache_dir() -> Result<PathBuf> {
    Ok(codex_utils_home_dir::find_codex_home()?
        .join("latex-cache")
        .join(CACHE_VERSION)
        .to_path_buf())
}

pub(crate) fn terminal_cell_pixels() -> (u16, u16) {
    crossterm::terminal::window_size()
        .ok()
        .and_then(|size| {
            let width = size.width.checked_div(size.columns)?;
            let height = size.height.checked_div(size.rows)?;
            (width > 0 && height > 0).then_some((width, height))
        })
        .unwrap_or((DEFAULT_CELL_WIDTH_PX, DEFAULT_CELL_HEIGHT_PX))
}

pub(super) fn cache_key(
    formula: &str,
    layout: FormulaLayout,
    available_columns: usize,
    cell_width_px: u16,
    cell_height_px: u16,
    foreground: (u8, u8, u8),
) -> String {
    let mut digest = Sha256::new();
    digest.update(CACHE_VERSION);
    digest.update(layout.cache_tag());
    digest.update(formula);
    if layout == FormulaLayout::Inline {
        digest.update(available_columns.to_le_bytes());
    }
    digest.update(cell_width_px.to_le_bytes());
    digest.update(cell_height_px.to_le_bytes());
    digest.update([foreground.0, foreground.1, foreground.2]);
    format!("{:x}", digest.finalize())
}

fn image_id_from_key(key: &str) -> u32 {
    let bytes = key.as_bytes();
    let value =
        u32::from_str_radix(std::str::from_utf8(&bytes[..6]).unwrap_or("1"), 16).unwrap_or(1);
    value.max(1)
}

pub(super) fn div_ceil_u32(value: u32, divisor: u32) -> u32 {
    value.saturating_add(divisor.saturating_sub(1)) / divisor
}
