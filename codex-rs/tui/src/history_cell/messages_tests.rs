use super::*;
use crate::history_cell::markdown_render_cache::MarkdownRenderCacheKey;
use pretty_assertions::assert_eq;

fn replace_cached_lines(
    cell: &AgentMarkdownCell,
    update_key: impl FnOnce(&mut MarkdownRenderCacheKey),
) {
    let rendered_lines = cell
        .rendered_lines
        .as_ref()
        .expect("ordinary markdown should be cacheable");
    let mut rendered_lines = rendered_lines.cached.lock().expect("render cache lock");
    let (key, lines) = rendered_lines
        .as_mut()
        .expect("render cache should be populated");
    *lines = vec![HyperlinkLine::from("cached")];
    update_key(key);
}

#[test]
fn finalized_markdown_reuses_lines_primed_by_transcript_height() {
    let cell = AgentMarkdownCell::new("finalized **markdown**".to_string(), Path::new("/tmp"));
    let width = 48;

    assert_eq!(cell.desired_transcript_height(width), 1);
    replace_cached_lines(&cell, |_| {});

    assert_eq!(
        visible_lines(cell.transcript_hyperlink_lines(width)),
        vec![Line::from("cached")]
    );
}

#[test]
fn finalized_markdown_cache_misses_when_geometry_or_render_style_changes() {
    let cell = AgentMarkdownCell::new("finalized **markdown**".to_string(), Path::new("/tmp"));
    let width = 48;
    let expected = cell.display_lines(width);

    replace_cached_lines(&cell, |key| key.width = key.width.saturating_sub(1));
    assert_eq!(cell.display_lines(width), expected);

    replace_cached_lines(&cell, |key| {
        key.terminal_cell_pixels.0 ^= 1;
    });
    assert_eq!(cell.display_lines(width), expected);

    replace_cached_lines(&cell, |key| {
        key.syntax_theme_revision = key.syntax_theme_revision.wrapping_sub(1);
    });
    assert_eq!(cell.display_lines(width), expected);

    replace_cached_lines(&cell, |key| {
        key.terminal_fg = key
            .terminal_fg
            .map_or(Some((1, 2, 3)), |(r, g, b)| Some((r ^ 1, g, b)));
    });
    assert_eq!(cell.display_lines(width), expected);
}

#[test]
fn raw_markdown_bypasses_the_rich_render_cache() {
    let source = "finalized **markdown**";
    let cell = AgentMarkdownCell::new(source.to_string(), Path::new("/tmp"));
    let width = 48;

    cell.display_lines(width);
    replace_cached_lines(&cell, |_| {});

    assert_eq!(
        cell.display_lines_for_mode(width, HistoryRenderMode::Raw),
        vec![Line::from(source)]
    );
}

#[test]
fn visualization_directives_are_not_cached() {
    let cell = AgentMarkdownCell::new(
        "::codex-inline-vis{file=\"chart.html\"}".to_string(),
        Path::new("/tmp"),
    );

    cell.display_lines(/*width*/ 48);

    assert!(cell.rendered_lines.is_none());
}

#[test]
fn finalized_mermaid_uses_fixture_placeholder_rows_with_assistant_gutters() {
    let temp = tempfile::tempdir().expect("temporary image directory");
    let image_path = temp.path().join("diagram.png");
    image::RgbaImage::from_pixel(30, 25, image::Rgba([255, 255, 255, 255]))
        .save(&image_path)
        .expect("write PNG fixture");
    let cell = AgentMarkdownCell::new(
        "Before\n```mermaid\ngraph TD\n  A --> B\n```\nAfter\n".to_string(),
        Path::new("/tmp"),
    );

    let lines = cell.rich_body_hyperlink_lines(
        /*width*/ 12,
        |markdown, wrap_width, render_markdown| {
            crate::mermaid_render::render_markdown_with_mermaid_using(
                markdown,
                wrap_width,
                render_markdown,
                |_, available_columns| {
                    crate::latex_render::render_cached_display_png_with_metrics(
                        &image_path,
                        "finalized-mermaid-fixture",
                        available_columns,
                        /*cell_width_px*/ 10,
                        /*cell_height_px*/ 20,
                        /*display_width_scale_halves*/ 2,
                    )
                },
            )
        },
    );

    let rendered = visible_lines(lines)
        .iter()
        .map(Line::to_string)
        .collect::<Vec<_>>()
        .join("\n");
    insta::assert_snapshot!(format!("{rendered:?}"), @r###""• Before\n  \u{10eeee}\u{305}\u{10eeee}\n  After""###);
}
