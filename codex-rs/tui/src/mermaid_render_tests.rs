use super::*;
use pretty_assertions::assert_eq;

#[test]
fn parser_accepts_complete_top_level_backtick_and_tilde_fences() {
    let markdown =
        "Before\n```Mermaid\ngraph TD\n  A-->B\n```\nAfter\n~~~mermaid\ngraph LR\n  C-->D\n~~~\n";
    assert_eq!(
        parse_mermaid_blocks(markdown),
        vec![
            MarkdownSegment::Markdown("Before\n"),
            MarkdownSegment::Mermaid {
                source: "```Mermaid\ngraph TD\n  A-->B\n```\n",
                diagram: "graph TD\n  A-->B\n",
            },
            MarkdownSegment::Markdown("After\n"),
            MarkdownSegment::Mermaid {
                source: "~~~mermaid\ngraph LR\n  C-->D\n~~~\n",
                diagram: "graph LR\n  C-->D\n",
            },
        ]
    );
}

#[test]
fn parser_and_renderer_accept_requested_indented_raw_diagrams() {
    let markdown = concat!(
        "***### Class diagram***\n",
        "  classDiagram\n",
        "      Animal <|-- Dog\n",
        "\n",
        "      Animal : +name\n",
        "      Animal : +eat()\n",
        "      Dog : +bark()\n",
        "  ***### Entity-relationship diagram***\n",
        "  erDiagram\n",
        "      USER ||--o{ ORDER : places\n",
        "\n",
        "      ORDER ||--|{ ITEM : contains\n",
        "  ***### Timeline***\n",
        "  timeline\n",
        "      title Project Timeline\n",
        "      2026-01 : Planning\n",
        "      2026-02 : Development\n",
        "      2026-03 : Launch\n",
        "  ***### Git graph***\n",
        "  gitGraph\n",
        "      commit\n",
        "      commit\n",
        "      branch feature\n",
        "      checkout feature\n",
        "      commit\n",
        "      checkout main\n",
        "      merge feature\n",
        "      commit\n",
    );
    let rendered_diagrams = std::cell::RefCell::new(Vec::new());

    render_markdown_with_mermaid_using(markdown, Some(80), &|_| Vec::new(), |diagram, _| {
        rendered_diagrams.borrow_mut().push(diagram.to_owned());
        Ok(vec![HyperlinkLine::new("IMAGE".into())])
    });

    assert_eq!(
        rendered_diagrams.into_inner(),
        vec![
            "classDiagram\n      Animal <|-- Dog\n\n      Animal : +name\n      Animal : +eat()\n      Dog : +bark()\n",
            "erDiagram\n      USER ||--o{ ORDER : places\n\n      ORDER ||--|{ ITEM : contains\n",
            "timeline\n      title Project Timeline\n      2026-01 : Planning\n      2026-02 : Development\n      2026-03 : Launch\n",
            "gitGraph\n      commit\n      commit\n      branch feature\n      checkout feature\n      commit\n      checkout main\n      merge feature\n      commit\n",
        ],
    );
}

#[test]
fn parser_recognizes_all_requested_indented_raw_headers() {
    let diagrams = [
        ("flowchart TD", "  A --> B"),
        ("sequenceDiagram", "  A->>B: request"),
        ("stateDiagram-v2", "  [*] --> Ready"),
        ("classDiagram", "  Animal <|-- Dog"),
        ("erDiagram", "  USER ||--o{ ORDER : places"),
        ("gantt", "  dateFormat YYYY-MM-DD"),
        ("timeline", "  2026-01 : Planning"),
        ("pie title Pets", "  \"Dogs\" : 3"),
        ("xychart-beta", "  x-axis [a, b]"),
        ("mindmap", "  root((Project))"),
        ("quadrantChart", "  x-axis Low --> High"),
        ("gitGraph", "  commit"),
    ];
    let markdown = diagrams
        .iter()
        .map(|(header, body)| format!("{header}\n{body}\n"))
        .collect::<String>();

    let parsed = parse_mermaid_blocks(&markdown);
    assert_eq!(
        parsed
            .iter()
            .filter_map(|segment| match segment {
                MarkdownSegment::Mermaid { diagram, .. } => diagram.lines().next(),
                MarkdownSegment::Markdown(_) => None,
            })
            .collect::<Vec<_>>(),
        diagrams
            .iter()
            .map(|(header, _)| *header)
            .collect::<Vec<_>>(),
    );
}

#[test]
fn raw_header_detection_rejects_invalid_prefixes_and_directions() {
    for markdown in [
        "flowchartTD\n  A --> B\n",
        "flowchart UU\n  A --> B\n",
        "piechart\n  \"Dogs\" : 3\n",
    ] {
        assert!(
            parse_mermaid_blocks(markdown)
                .iter()
                .all(|segment| matches!(segment, MarkdownSegment::Markdown(_))),
            "unexpected raw Mermaid block: {markdown}",
        );
    }
}

#[test]
fn raw_diagram_limit_preserves_the_thirteenth_source_as_markdown() {
    let markdown = (1..=13)
        .map(|number| format!("timeline\n  item {number}\n"))
        .collect::<String>();
    let rendered = std::cell::Cell::new(0);
    let fallback = std::cell::RefCell::new(Vec::new());

    render_markdown_with_mermaid_using(
        &markdown,
        Some(80),
        &|source| {
            fallback.borrow_mut().push(source.to_owned());
            Vec::new()
        },
        |_, _| {
            rendered.set(rendered.get() + 1);
            Ok(vec![HyperlinkLine::new("IMAGE".into())])
        },
    );

    assert_eq!(rendered.get(), 12);
    assert_eq!(fallback.into_inner(), vec!["timeline\n  item 13\n"]);
}

#[test]
fn parser_preserves_ineligible_or_incomplete_fences() {
    let markdown = concat!(
        "timeline\nordinary prose\n",
        "classDiagram\n",
        "  ```mermaid\ngraph TD\n  ```\n",
        "    ```mermaid\ngraph TD\n    ```\n",
        "> ```mermaid\n> graph TD\n> ```\n",
        "```text\nerDiagram\n  USER ||--o{ ORDER : places\n```\n",
        "````text\n```mermaid\ngraph TD\n```\n````\n",
        "```mermaid extra\ngraph TD\n```\n",
        "```mermaid\ngraph TD\n",
    );
    assert_eq!(
        parse_mermaid_blocks(markdown),
        vec![MarkdownSegment::Markdown(markdown)]
    );
    assert!(!contains_mermaid_diagram(markdown));
}

#[test]
fn a_failed_block_falls_back_without_stopping_later_blocks() {
    let markdown = "```mermaid\nfirst\n```\ntext\n```mermaid\nsecond\n```\n";
    let fallback_sources = std::cell::RefCell::new(Vec::new());
    let rendered = render_markdown_with_mermaid_using(
        markdown,
        Some(80),
        &|source| {
            fallback_sources.borrow_mut().push(source.to_owned());
            vec![HyperlinkLine::from(source.to_owned())]
        },
        |diagram, _| {
            if diagram.starts_with("first") {
                anyhow::bail!("intentional")
            }
            Ok(vec![HyperlinkLine::new("IMAGE".into())])
        },
    );
    assert_eq!(
        fallback_sources.into_inner(),
        vec!["```mermaid\nfirst\n```\n", "text\n"]
    );
    assert_eq!(
        rendered
            .iter()
            .map(|line| line.line.to_string())
            .collect::<Vec<_>>(),
        vec!["```mermaidfirst```", "text", "IMAGE"]
    );
}

#[test]
fn unsafe_or_empty_diagrams_fail_before_renderer_resolution() {
    assert!(render_mermaid("\n", 80).is_err());
    assert!(render_mermaid("%%{init: { 'theme': 'forest' }}%%\ngraph TD\nA-->B", 80).is_err());
}

#[test]
fn rendered_mermaid_png_uses_kitty_placeholder_grid() {
    let temp = tempfile::tempdir().expect("temporary cache");
    let path = temp.path().join("diagram.png");
    image::RgbaImage::from_pixel(30, 25, image::Rgba([255, 255, 255, 255]))
        .save(&path)
        .expect("write PNG fixture");

    let lines = crate::latex_render::render_cached_display_png_with_metrics(
        &path,
        "mermaid-placeholder-snapshot",
        /*available_columns*/ 10,
        /*cell_width_px*/ 10,
        /*cell_height_px*/ 20,
        /*display_width_scale_halves*/ 4,
    )
    .expect("render cached Mermaid PNG");

    insta::assert_debug_snapshot!("mermaid_placeholder_grid", lines);
}

#[test]
fn rendered_mermaid_png_fits_wide_images_without_horizontal_tiles() {
    let temp = tempfile::tempdir().expect("temporary cache");
    let path = temp.path().join("wide-diagram.png");
    image::RgbaImage::from_pixel(300, 25, image::Rgba([255, 255, 255, 255]))
        .save(&path)
        .expect("write PNG fixture");

    let lines = crate::latex_render::render_cached_display_png_with_metrics(
        &path,
        "wide-mermaid-placeholder",
        /*available_columns*/ 10,
        /*cell_width_px*/ 10,
        /*cell_height_px*/ 20,
        /*display_width_scale_halves*/ 4,
    )
    .expect("render cached Mermaid PNG");

    assert_eq!(
        lines.len(),
        1,
        "wide image must not create continuation tiles"
    );
    assert!(lines.iter().all(|line| {
        line.line
            .to_string()
            .chars()
            .filter(|character| *character == '\u{10eeee}')
            .count()
            <= 10
    }));
}

#[test]
fn rendered_mermaid_png_shrinks_with_terminal_cells() {
    let temp = tempfile::tempdir().expect("temporary cache");
    let path = temp.path().join("zoom-diagram.png");
    image::RgbaImage::from_pixel(100, 40, image::Rgba([255, 255, 255, 255]))
        .save(&path)
        .expect("write PNG fixture");

    let render = |cache_key, cell_width_px, cell_height_px| {
        crate::latex_render::render_cached_display_png_with_metrics(
            &path,
            cache_key,
            /*available_columns*/ 40,
            cell_width_px,
            cell_height_px,
            /*display_width_scale_halves*/ 4,
        )
        .expect("render cached Mermaid PNG")
    };
    let normal = render("normal-mermaid-zoom", 10, 20);
    let zoomed_out = render("small-mermaid-zoom", 5, 10);
    let zoomed_in = render("large-mermaid-zoom", 20, 40);

    let dimensions = |lines: &[HyperlinkLine]| {
        (
            lines.first().map_or(0, |line| {
                line.line
                    .to_string()
                    .chars()
                    .filter(|character| *character == '\u{10eeee}')
                    .count()
            }),
            lines.len(),
        )
    };

    assert_eq!(dimensions(&normal), (5, 1));
    assert_eq!(dimensions(&zoomed_out), dimensions(&normal));
    assert_eq!(dimensions(&zoomed_in), dimensions(&normal));
}

#[test]
fn dense_near_square_mermaid_png_uses_native_width_without_cutting() {
    let temp = tempfile::tempdir().expect("temporary cache");
    let path = temp.path().join("dense-diagram.png");
    image::RgbaImage::from_pixel(100, 90, image::Rgba([255, 255, 255, 255]))
        .save(&path)
        .expect("write PNG fixture");

    let render = |cache_key, cell_width_px, cell_height_px| {
        crate::latex_render::render_cached_display_png_with_metrics(
            &path,
            cache_key,
            /*available_columns*/ 40,
            cell_width_px,
            cell_height_px,
            /*display_width_scale_halves*/ 4,
        )
        .expect("render cached Mermaid PNG")
    };
    let dimensions = |lines: &[HyperlinkLine]| {
        (
            lines.first().map_or(0, |line| {
                line.line
                    .to_string()
                    .chars()
                    .filter(|character| *character == '\u{10eeee}')
                    .count()
            }),
            lines.len(),
        )
    };

    assert_eq!(dimensions(&render("normal-dense-mermaid", 10, 20)), (10, 5));
    assert_eq!(dimensions(&render("small-dense-mermaid", 5, 10)), (10, 5));
    assert_eq!(dimensions(&render("large-dense-mermaid", 20, 40)), (5, 3));
}

#[test]
fn near_square_flowchart_png_uses_compact_width() {
    let temp = tempfile::tempdir().expect("temporary cache");
    let path = temp.path().join("flowchart.png");
    image::RgbaImage::from_pixel(100, 90, image::Rgba([255, 255, 255, 255]))
        .save(&path)
        .expect("write PNG fixture");

    let lines = crate::latex_render::render_cached_display_png_with_metrics(
        &path,
        "compact-flowchart",
        /*available_columns*/ 40,
        /*cell_width_px*/ 10,
        /*cell_height_px*/ 20,
        /*display_width_scale_halves*/ 2,
    )
    .expect("render compact flowchart PNG");
    let columns = lines.first().map_or(0, |line| {
        line.line
            .to_string()
            .chars()
            .filter(|character| *character == '\u{10eeee}')
            .count()
    });

    assert_eq!((columns, lines.len()), (5, 3));
}

#[test]
fn near_square_class_diagram_uses_compact_width() {
    let temp = tempfile::tempdir().expect("temporary cache");
    let path = temp.path().join("class-diagram.png");
    image::RgbaImage::from_pixel(100, 90, image::Rgba([255, 255, 255, 255]))
        .save(&path)
        .expect("write PNG fixture");

    let lines = crate::latex_render::render_cached_display_png_with_metrics(
        &path,
        "compact-class-diagram",
        /*available_columns*/ 40,
        /*cell_width_px*/ 10,
        /*cell_height_px*/ 20,
        /*display_width_scale_halves*/ 2,
    )
    .expect("render compact class diagram PNG");
    let columns = lines.first().map_or(0, |line| {
        line.line
            .to_string()
            .chars()
            .filter(|character| *character == '\u{10eeee}')
            .count()
    });

    assert_eq!((columns, lines.len()), (5, 3));
}
