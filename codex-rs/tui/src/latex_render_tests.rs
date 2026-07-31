use image::ImageFormat;
use image::RgbaImage;
use pretty_assertions::assert_eq;

use super::*;

#[test]
fn display_math_parser_preserves_surrounding_markdown() {
    let markdown = "Before\n\n$$\nx^2 + y^2 = z^2\n$$\n\nAfter\n";

    assert_eq!(
        parse_display_math(markdown),
        vec![
            MarkdownSegment::Markdown("Before\n\n"),
            MarkdownSegment::DisplayMath {
                source: "$$\nx^2 + y^2 = z^2\n$$\n",
                formula: "x^2 + y^2 = z^2",
            },
            MarkdownSegment::Markdown("\nAfter\n"),
        ]
    );
}

#[test]
fn display_math_parser_accepts_bracket_delimiters() {
    let markdown = "Before\n\n\\[\nx^2 + y^2 = z^2\n\\]\n\nAfter\n";

    assert_eq!(
        parse_display_math(markdown),
        vec![
            MarkdownSegment::Markdown("Before\n\n"),
            MarkdownSegment::DisplayMath {
                source: "\\[\nx^2 + y^2 = z^2\n\\]\n",
                formula: "x^2 + y^2 = z^2",
            },
            MarkdownSegment::Markdown("\nAfter\n"),
        ]
    );
}

#[test]
fn display_math_parser_accepts_indented_bracket_delimiters() {
    let markdown = "Before\n\n  \\[\n  E = mc^2\n  \\]\n\nAfter\n";

    assert_eq!(
        parse_display_math(markdown),
        vec![
            MarkdownSegment::Markdown("Before\n\n"),
            MarkdownSegment::DisplayMath {
                source: "  \\[\n  E = mc^2\n  \\]\n",
                formula: "E = mc^2",
            },
            MarkdownSegment::Markdown("\nAfter\n"),
        ]
    );
}

#[test]
fn display_math_parser_accepts_full_indented_user_batch() {
    let markdown = concat!(
        "  E = mc^2\n\n",
        "  \\[\n  \\frac{a}{b} + \\frac{c}{d}\n  \\]\n\n",
        "  \\[\n  x = \\frac{-b \\pm \\sqrt{b^2 - 4ac}}{2a}\n  \\]\n\n",
        "  \\[\n  \\begin{aligned}\n  y &= mx + b \\\\\n",
        "  E &= mc^2 \\\\\n  a^2 + b^2 &= c^2\n  \\end{aligned}\n  \\]\n\n",
        "  \\[\n  \\int_0^1 x^2\\,dx = \\frac{1}{3}\n  \\]\n\n",
        "  \\[\n  \\underbrace{x + y + z}_{\\text{sum of three variables}}\n  \\]\n",
    );

    let formulas = parse_display_math(markdown)
        .into_iter()
        .filter_map(|segment| match segment {
            MarkdownSegment::DisplayMath { formula, .. } => Some(formula),
            MarkdownSegment::Markdown(_) => None,
        })
        .collect::<Vec<_>>();

    assert_eq!(
        formulas,
        vec![
            r"\frac{a}{b} + \frac{c}{d}",
            r"x = \frac{-b \pm \sqrt{b^2 - 4ac}}{2a}",
            "\\begin{aligned}\n  y &= mx + b \\\\\n  E &= mc^2 \\\\\n  a^2 + b^2 &= c^2\n  \\end{aligned}",
            r"\int_0^1 x^2\,dx = \frac{1}{3}",
            r"\underbrace{x + y + z}_{\text{sum of three variables}}",
        ],
    );
}

#[test]
fn display_math_parser_accepts_same_line_delimiters() {
    let markdown = "$$E = mc^2$$\n\n\\[e^{i\\pi}+1=0\\]\n\n$$\nA = B\n$$\n";

    assert_eq!(
        parse_display_math(markdown),
        vec![
            MarkdownSegment::DisplayMath {
                source: "$$E = mc^2$$\n",
                formula: "E = mc^2",
            },
            MarkdownSegment::Markdown("\n"),
            MarkdownSegment::DisplayMath {
                source: "\\[e^{i\\pi}+1=0\\]\n",
                formula: "e^{i\\pi}+1=0",
            },
            MarkdownSegment::Markdown("\n"),
            MarkdownSegment::DisplayMath {
                source: "$$\nA = B\n$$\n",
                formula: "A = B",
            },
        ]
    );
}

#[test]
fn display_math_parser_leaves_unclosed_block_untouched() {
    let markdown = "Before\n\n$$\nx + y\n";

    assert_eq!(
        parse_display_math(markdown),
        vec![
            MarkdownSegment::Markdown("Before\n\n"),
            MarkdownSegment::Markdown("$$\nx + y\n"),
        ]
    );
}

#[test]
fn display_math_parser_ignores_dollars_inside_code_fence() {
    let markdown = "```md\n$$\nnot math\n$$\n```\n";

    assert_eq!(
        parse_display_math(markdown),
        vec![MarkdownSegment::Markdown(markdown)]
    );
}

#[test]
fn display_math_detection_requires_a_closed_unfenced_block() {
    assert_eq!(
        (
            contains_latex_math("$$\nx + y\n$$\n"),
            contains_latex_math("$$x + y$$\n"),
            contains_latex_math("\\[\nx + y\n\\]\n"),
            contains_latex_math("\\[x + y\\]\n"),
            contains_latex_math("$$\nx + y\n"),
            contains_latex_math("\\[\nx + y\n"),
            contains_latex_math("```md\n$$\nx + y\n$$\n```\n"),
        ),
        (true, true, true, true, false, false, false),
    );
}

#[test]
fn inline_math_parser_extracts_dollar_and_parenthesized_forms() {
    let markdown = "Energy $E = mc^2$ and \\(e^{i\\pi}+1=0\\).";

    assert_eq!(
        inline::parse_inline_math(markdown),
        vec![
            inline::InlineMathToken {
                source: 7..17,
                formula: "E = mc^2",
            },
            inline::InlineMathToken {
                source: 22..38,
                formula: "e^{i\\pi}+1=0",
            },
        ]
    );
    assert!(contains_latex_math(markdown));
}

#[test]
fn inline_math_parser_ignores_escaped_code_link_display_and_unclosed_forms() {
    let markdown = concat!(
        "Price: \\$5 and literal `$x$`.\n",
        "```latex\n\\(y\\) and $z$\n```\n",
        "[$linked$](https://example.com)\n",
        "$$display$$\n",
        "$ unspaced$\n",
        "$unclosed\n",
        "\\(also unclosed\n",
        "Render $x$ and \\(y\\).\n",
    );

    assert_eq!(
        inline::parse_inline_math(markdown)
            .into_iter()
            .map(|token| token.formula)
            .collect::<Vec<_>>(),
        vec!["x", "y"],
    );
}

#[test]
fn inline_math_parser_supports_multiple_formulas_on_one_line() {
    assert_eq!(
        inline::parse_inline_math("$x$ + $y$ = $z$")
            .into_iter()
            .map(|token| token.formula)
            .collect::<Vec<_>>(),
        vec!["x", "y", "z"],
    );
}

#[test]
fn inline_math_parser_supports_tight_list_items() {
    let markdown = concat!(
        "- $\\phi_1,\\phi_2$ are latitudes in radians\n",
        "- $\\lambda_1,\\lambda_2$ are longitudes in radians\n",
        "- $R$ is Earth’s radius, approximately $6{,}371$ km or $3{,}959$ miles\n",
        "- $d$ is the surface distance between the points\n",
    );

    assert_eq!(
        inline::parse_inline_math(markdown)
            .into_iter()
            .map(|token| token.formula)
            .collect::<Vec<_>>(),
        vec![
            "\\phi_1,\\phi_2",
            "\\lambda_1,\\lambda_2",
            "R",
            "6{,}371",
            "3{,}959",
            "d",
        ],
    );
}

#[test]
fn inline_math_parser_preserves_protected_regions_in_tight_list_items() {
    let markdown = concat!(
        "- Inline code `$code$` stays literal\n",
        "- A [$link$](https://example.com) stays literal\n",
        "- An ![$image$](image.png) stays literal\n",
        "- An attribute <span data-math=\"$attribute$\">stays literal</span>\n",
        "- A [$reference$][ref] stays literal\n",
        "- Currency $25. stays literal\n",
        "- $$display$$ stays display math\n",
        "- Visible $y$ renders\n\n",
        "[ref]: https://example.com/$destination$\n",
    );

    assert_eq!(
        inline::parse_inline_math(markdown)
            .into_iter()
            .map(|token| token.formula)
            .collect::<Vec<_>>(),
        vec!["y"],
    );
}

#[test]
fn inline_math_parser_ignores_reference_destinations_and_adjacent_display_dollars() {
    let markdown = concat!(
        "[$label$][ref]\n\n",
        "[ref]: https://example.com/$destination$\n",
        "Bare https://example.com/$path$ stays a URL.\n",
        "$a$$b$ is not inline math.\n",
        "But $x$ is.\n",
    );

    assert_eq!(
        inline::parse_inline_math(markdown)
            .into_iter()
            .map(|token| token.formula)
            .collect::<Vec<_>>(),
        vec!["path", "x"],
    );
    let url_source = markdown.find("$path$").expect("URL formula");
    assert!(!inline::replacement_fits(
        markdown,
        &(url_source..url_source + "$path$".len()),
        "x://\u{e000}\u{e000}",
        80,
    ));
}

#[test]
fn inline_math_parser_ignores_reference_destinations_nested_in_containers() {
    let markdown = concat!(
        "- [$label$][list]\n\n",
        "  [list]: https://example.com/$hidden_list$\n\n",
        "> [$quoted$][quote]\n>\n",
        "> [quote]: https://example.com/$hidden_quote$\n\n",
        "| visible |\n",
        "| --- |\n",
        "| \\(table_math\\) |\n",
    );

    assert_eq!(
        inline::parse_inline_math(markdown)
            .into_iter()
            .map(|token| token.formula)
            .collect::<Vec<_>>(),
        vec!["table_math"],
    );
}

#[test]
fn placeholder_grid_encodes_row_and_image_id() {
    let rendered = RenderedFormula {
        image_id: 0x123456,
        columns: 3,
        rows: 2,
        upload_path: None,
    };

    let lines = placeholder_lines(&rendered);

    assert_eq!(lines.len(), 2);
    assert_eq!(
        lines[0].line.spans,
        vec![Span::styled(
            format!("{PLACEHOLDER}{}{PLACEHOLDER}{PLACEHOLDER}", DIACRITICS[0]),
            Style::default().fg(rgb_color((0x12, 0x34, 0x56))),
        )]
    );
    assert_eq!(
        lines[1].line.spans,
        vec![Span::styled(
            format!("{PLACEHOLDER}{}{PLACEHOLDER}{PLACEHOLDER}", DIACRITICS[1]),
            Style::default().fg(rgb_color((0x12, 0x34, 0x56))),
        )]
    );
    insta::assert_debug_snapshot!("latex_placeholder_grid", lines);
}

#[test]
fn latex_document_disables_page_chrome_and_uses_display_math() {
    let document = latex_document(r"\frac{a}{b}", FormulaLayout::Display);

    assert!(document.contains(r"\pagestyle{empty}"));
    assert!(document.contains("\\[\n\\frac{a}{b}\n\\]"));
}

#[test]
fn latex_document_uses_inline_math_without_display_style() {
    let document = latex_document(r"\frac{a}{b}", FormulaLayout::Inline);

    assert!(document.contains(r"\noindent\(\frac{a}{b}\)"));
    assert!(!document.contains("\\[\n"));
}

#[test]
fn inline_placeholder_preserves_neighbor_styles_and_width() {
    use ratatui::style::Modifier;

    let rendered = RenderedFormula {
        image_id: 0x123456,
        columns: 3,
        rows: 1,
        upload_path: None,
    };
    let source = HyperlinkLine::new(Line::from(vec![
        Span::styled("before ", Style::default().bold()),
        Span::styled("\u{e000}\u{e000}\u{e000}", Style::default().italic()),
        Span::styled(" after", Style::default().underlined()),
    ]));
    let original_width = source.width();

    let lines = inline::replace_inline_sentinels(
        vec![source],
        &[inline::InlineReplacement {
            source: 0..0,
            token: "\u{e000}\u{e000}\u{e000}".to_string(),
            rendered,
        }],
    )
    .expect("replace complete sentinel");

    assert_eq!(lines[0].width(), original_width);
    assert_eq!(lines[0].line.spans[0].style, Style::default().bold());
    assert_eq!(
        lines[0].line.spans[1].style,
        Style::default().italic().fg(rgb_color((0x12, 0x34, 0x56))),
    );
    assert_eq!(lines[0].line.spans[1].style.add_modifier, Modifier::ITALIC);
    assert_eq!(
        lines[0].line.spans[1].content,
        format!("{PLACEHOLDER}{}{PLACEHOLDER}{PLACEHOLDER}", DIACRITICS[0]),
    );
    assert_eq!(lines[0].line.spans[2].style, Style::default().underlined());
    insta::assert_debug_snapshot!("latex_inline_placeholder", lines);
}

#[test]
fn inline_sentinel_wraps_as_one_atomic_token() {
    let sentinel = format!("x://{}", "\u{e000}".repeat(3));
    let line = Line::from(format!("Let {sentinel}, then y"));
    let wrapped =
        crate::wrapping::adaptive_wrap_line(&line, crate::wrapping::RtOptions::new(/*width*/ 10));
    let rendered = wrapped
        .iter()
        .map(|line| {
            line.spans
                .iter()
                .map(|span| span.content.as_ref())
                .collect::<String>()
        })
        .collect::<Vec<_>>();

    assert_eq!(
        rendered
            .iter()
            .filter(|line| line.contains(&sentinel))
            .count(),
        1,
    );
    assert!(
        rendered
            .iter()
            .all(|line| !line.contains("x://") || line.contains(&sentinel)),
    );
}

#[test]
fn invalid_inline_sentinel_falls_back_without_disabling_other_formulas() {
    let markdown = "A $x$ and $y$.";
    let first_start = markdown.find("$x$").expect("first formula");
    let second_start = markdown.find("$y$").expect("second formula");
    let rendered = RenderedFormula {
        image_id: 0x123456,
        columns: 2,
        rows: 1,
        upload_path: None,
    };
    let mut replacements = vec![
        inline::InlineReplacement {
            source: first_start..first_start + 3,
            token: "\u{e000}\u{e000}".to_string(),
            rendered: rendered.clone(),
        },
        inline::InlineReplacement {
            source: second_start..second_start + 3,
            token: "\u{e001}\u{e001}".to_string(),
            rendered,
        },
    ];
    let lines = vec![HyperlinkLine::from(format!(
        "A split and {}.",
        replacements[1].token
    ))];

    let invalid = inline::invalid_replacement_indices(&lines, &replacements);
    assert_eq!(invalid, vec![0]);
    inline::remove_replacements(&mut replacements, &invalid);
    assert_eq!(
        inline::rewrite_with_replacements(markdown, &replacements),
        "A $x$ and \u{e001}\u{e001}.",
    );
}

#[test]
fn display_post_processing_preserves_full_size_at_different_widths() {
    let temp = tempfile::tempdir().expect("temporary cache");
    let source = temp.path().join("source.png");
    let narrow = temp.path().join("narrow.png");
    let wide = temp.path().join("wide.png");
    let mut fixture = RgbaImage::new(420, 135);
    fixture.put_pixel(0, 0, image::Rgba([255, 0, 0, 255]));
    fixture.put_pixel(137, 52, image::Rgba([0, 255, 0, 192]));
    fixture.put_pixel(419, 134, image::Rgba([0, 0, 255, 255]));
    fixture
        .save_with_format(&source, ImageFormat::Png)
        .expect("write fixture");

    post_process_png(
        &source,
        &narrow,
        FormulaLayout::Display,
        /*available_columns*/ 40,
        /*cell_width_px*/ 10,
        /*cell_height_px*/ 20,
    )
    .expect("post-process narrow");
    post_process_png(
        &source,
        &wide,
        FormulaLayout::Display,
        /*available_columns*/ 120,
        /*cell_width_px*/ 10,
        /*cell_height_px*/ 20,
    )
    .expect("post-process wide");

    let narrow = image::open(narrow).expect("read narrow");
    let wide = image::open(wide).expect("read wide");
    assert_eq!(
        (narrow.width(), narrow.height()),
        (wide.width(), wide.height())
    );
    assert_eq!((narrow.width(), narrow.height()), (430, 143));
    assert_eq!(narrow.to_rgba8(), wide.to_rgba8());
}

#[test]
fn display_tiling_does_not_clamp_columns_to_diacritic_count() {
    let temp = tempfile::tempdir().expect("temporary cache");
    let cache_path = temp.path().join("formula.png");
    let image = RgbaImage::from_pixel(700, 80, image::Rgba([255, 255, 255, 255]));

    let rendered = tile_display_formula(
        &image,
        &cache_path,
        "wide-fixture",
        /*available_columns*/ 80,
        /*cell_width_px*/ 10,
        /*cell_height_px*/ 20,
    )
    .expect("tile image");

    assert_eq!(rendered.len(), 1);
    assert_eq!(rendered[0].columns, 70);
    assert_eq!(rendered[0].rows, 4);
    let tile = image::open(temp.path().join("wide-fixture-c80-x0-y0.png")).expect("read tile");
    assert_eq!((tile.width(), tile.height()), (700, 80));
}

#[test]
fn display_tiling_stacks_wide_images_without_resampling() {
    let temp = tempfile::tempdir().expect("temporary cache");
    let cache_path = temp.path().join("formula.png");
    let image = RgbaImage::from_pixel(950, 80, image::Rgba([255, 255, 255, 255]));

    let rendered = tile_display_formula(
        &image,
        &cache_path,
        "tiled-fixture",
        /*available_columns*/ 40,
        /*cell_width_px*/ 10,
        /*cell_height_px*/ 20,
    )
    .expect("tile image");

    assert_eq!(
        rendered
            .iter()
            .map(|tile| (tile.columns, tile.rows))
            .collect::<Vec<_>>(),
        vec![(40, 4), (40, 4), (15, 4)],
    );
    let first =
        image::open(temp.path().join("tiled-fixture-c40-x0-y0.png")).expect("read first tile");
    let last =
        image::open(temp.path().join("tiled-fixture-c40-x2-y0.png")).expect("read last tile");
    assert_eq!((first.width(), first.height()), (400, 80));
    assert_eq!((last.width(), last.height()), (150, 80));
}

#[test]
fn display_tiling_stacks_tall_images_at_the_row_diacritic_limit() {
    let temp = tempfile::tempdir().expect("temporary cache");
    let cache_path = temp.path().join("formula.png");
    let mut image = RgbaImage::new(400, 1281);
    image.put_pixel(0, 1279, image::Rgba([255, 0, 0, 255]));
    image.put_pixel(0, 1280, image::Rgba([0, 255, 0, 255]));

    let rendered = tile_display_formula(
        &image,
        &cache_path,
        "tall-fixture",
        /*available_columns*/ 40,
        /*cell_width_px*/ 10,
        /*cell_height_px*/ 20,
    )
    .expect("tile image");

    assert_eq!(
        rendered
            .iter()
            .map(|tile| (tile.columns, tile.rows))
            .collect::<Vec<_>>(),
        vec![(40, 64), (40, 1)],
    );
    let first = image::open(temp.path().join("tall-fixture-c40-x0-y0.png"))
        .expect("read first tile")
        .to_rgba8();
    let second = image::open(temp.path().join("tall-fixture-c40-x0-y1.png"))
        .expect("read second tile")
        .to_rgba8();
    assert_eq!((first.width(), first.height()), (400, 1280));
    assert_eq!((second.width(), second.height()), (400, 20));
    assert_eq!(first.get_pixel(0, 1279), &image::Rgba([255, 0, 0, 255]));
    assert_eq!(second.get_pixel(0, 0), &image::Rgba([0, 255, 0, 255]));
}

#[test]
fn inline_post_processing_allocates_rows_from_formula_height() {
    let temp = tempfile::tempdir().expect("temporary cache");
    let source = temp.path().join("source.png");
    let destination = temp.path().join("inline.png");
    RgbaImage::from_pixel(120, 80, image::Rgba([255, 255, 255, 255]))
        .save_with_format(&source, ImageFormat::Png)
        .expect("write fixture");

    post_process_png(
        &source,
        &destination,
        FormulaLayout::Inline,
        /*available_columns*/ 20,
        /*cell_width_px*/ 10,
        /*cell_height_px*/ 20,
    )
    .expect("post-process inline");

    let image = image::open(destination).expect("read inline");
    assert_eq!(image.height(), 20);
    assert!(image.width() <= 200);
    assert_eq!(image.width() % 10, 0);
    assert!(div_ceil_u32(image.width(), 10) <= 20);

    let short_source = temp.path().join("short-source.png");
    let short_destination = temp.path().join("short-inline.png");
    RgbaImage::from_pixel(120, 20, image::Rgba([255, 255, 255, 255]))
        .save_with_format(&short_source, ImageFormat::Png)
        .expect("write short fixture");
    post_process_png(
        &short_source,
        &short_destination,
        FormulaLayout::Inline,
        /*available_columns*/ 20,
        /*cell_width_px*/ 10,
        /*cell_height_px*/ 20,
    )
    .expect("post-process short inline");
    assert_eq!(
        image::open(short_destination)
            .expect("read short inline")
            .height(),
        20
    );

    let alpha_bounds = image.to_rgba8().enumerate_pixels().fold(
        None,
        |bounds: Option<(u32, u32)>, (_, y, pixel)| {
            (pixel[3] > 0)
                .then(|| bounds.map_or((y, y), |(min, max)| (min.min(y), max.max(y))))
                .or(bounds)
        },
    );
    assert_eq!(alpha_bounds, Some((3, 16)));
}

#[test]
fn cache_key_separates_display_and_inline_layouts() {
    let display = cache_key("x", FormulaLayout::Display, 80, 10, 20, (220, 220, 220));
    let inline = cache_key("x", FormulaLayout::Inline, 80, 10, 20, (220, 220, 220));

    assert_ne!(display, inline);
}

#[test]
fn image_id_allocation_reuses_owner_and_probes_collisions_in_rgb_range() {
    let mut owners = HashMap::new();
    owners.insert(0x123456, "first".to_string());

    assert_eq!(available_image_id(&owners, "first", 0x123456), 0x123456);
    assert_eq!(available_image_id(&owners, "second", 0x123456), 0x123457);

    owners.insert(0x00ff_ffff, "last".to_string());
    assert_eq!(available_image_id(&owners, "wrapped", 0x00ff_ffff), 1);
    owners.insert(1, "one".to_string());
    assert_eq!(available_image_id(&owners, "wrapped", 0x00ff_ffff), 2);
}

#[test]
#[serial_test::serial]
fn inline_upload_planning_keeps_global_state_unchanged_when_a_later_path_is_missing() {
    let temp = tempfile::tempdir().expect("temporary directory");
    let valid = temp.path().join("valid.png");
    std::fs::write(&valid, b"png").expect("write valid image");
    let missing = temp.path().join("missing.png");
    let mut replacements = vec![
        inline::InlineReplacement {
            source: 0..3,
            token: "\u{e000}".to_string(),
            rendered: RenderedFormula {
                image_id: 101,
                columns: 1,
                rows: 1,
                upload_path: Some(valid),
            },
        },
        inline::InlineReplacement {
            source: 4..7,
            token: "\u{e001}".to_string(),
            rendered: RenderedFormula {
                image_id: 102,
                columns: 1,
                rows: 1,
                upload_path: Some(missing),
            },
        },
    ];

    let uploads = UPLOADS.get_or_init(Default::default);
    let original = {
        let mut state = uploads
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let original = (
            state.owners.clone(),
            state.sent.clone(),
            state.pending.clone(),
        );
        *state = UploadState::default();
        original
    };

    let result = queue_inline_uploads_after(&mut replacements, |_| Some(()));

    {
        let state = uploads
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        assert!(state.owners.is_empty());
        assert!(state.pending.is_empty());
        assert!(state.sent.is_empty());
    }
    {
        let mut state = uploads
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        state.owners = original.0;
        state.sent = original.1;
        state.pending = original.2;
    }

    assert_eq!(result.expect_err("missing second image should fail").0, 1);
}

#[test]
#[serial_test::serial]
fn inline_upload_planning_keeps_global_state_unchanged_when_final_splice_fails() {
    let temp = tempfile::tempdir().expect("temporary directory");
    let valid = temp.path().join("valid.png");
    std::fs::write(&valid, b"png").expect("write valid image");
    let mut replacements = vec![inline::InlineReplacement {
        source: 0..3,
        token: "\u{e000}".to_string(),
        rendered: RenderedFormula {
            image_id: 101,
            columns: 1,
            rows: 1,
            upload_path: Some(valid),
        },
    }];

    let uploads = UPLOADS.get_or_init(Default::default);
    let original = {
        let mut state = uploads
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let original = (
            state.owners.clone(),
            state.sent.clone(),
            state.pending.clone(),
        );
        *state = UploadState::default();
        original
    };

    let result = queue_inline_uploads_after(&mut replacements, |replacements| {
        inline::replace_inline_sentinels(vec![HyperlinkLine::from("not a sentinel")], replacements)
    });

    assert!(result.expect("planning succeeds before splice").is_none());
    {
        let state = uploads
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        assert!(state.owners.is_empty());
        assert!(state.pending.is_empty());
        assert!(state.sent.is_empty());
    }
    {
        let mut state = uploads
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        state.owners = original.0;
        state.sent = original.1;
        state.pending = original.2;
    }
}

#[test]
fn installed_latex_toolchain_produces_a_valid_png() {
    let (Some(latex), Some(dvipng)) = (resolve_executable("latex"), resolve_executable("dvipng"))
    else {
        return;
    };
    let temp = tempfile::tempdir().expect("temporary cache");
    let destination = temp.path().join("formula.png");

    compile_formula(
        r"\int_0^\infty e^{-x^2}\,dx = \frac{\sqrt{\pi}}{2}",
        FormulaLayout::Display,
        &latex,
        &dvipng,
        &destination,
        /*available_columns*/ 60,
        /*cell_width_px*/ 10,
        /*cell_height_px*/ 20,
        /*foreground*/ (220, 220, 220),
    )
    .expect("render formula");

    let image = image::open(destination).expect("read rendered PNG");
    assert!(image.width() > 1);
    assert!(image.height() > 1);

    let matrix = temp.path().join("matrix.png");
    compile_formula(
        "\\mathbf{A} =\n\\begin{pmatrix}\na & b \\\\\nc & d\n\\end{pmatrix}",
        FormulaLayout::Display,
        &latex,
        &dvipng,
        &matrix,
        /*available_columns*/ 60,
        /*cell_width_px*/ 10,
        /*cell_height_px*/ 20,
        /*foreground*/ (220, 220, 220),
    )
    .expect("render matrix");
    let matrix = image::open(matrix).expect("read matrix PNG");
    assert!(matrix.width() > 1);
    assert!(matrix.height() > 1);

    for (index, formula) in [
        r"\frac{a}{b} + \frac{c}{d}",
        r"x = \frac{-b \pm \sqrt{b^2 - 4ac}}{2a}",
        "\\begin{aligned}\ny &= mx + b \\\\\nE &= mc^2 \\\\\na^2 + b^2 &= c^2\n\\end{aligned}",
        r"\int_0^1 x^2\,dx = \frac{1}{3}",
        r"\underbrace{x + y + z}_{\text{sum of three variables}}",
    ]
    .into_iter()
    .enumerate()
    {
        let destination = temp.path().join(format!("user-formula-{index}.png"));
        compile_formula(
            formula,
            FormulaLayout::Display,
            &latex,
            &dvipng,
            &destination,
            60,
            10,
            20,
            (220, 220, 220),
        )
        .unwrap_or_else(|err| panic!("render user formula {index}: {err:#}"));
        let image = image::open(destination).expect("read user formula PNG");
        assert!(image.width() > 1 && image.height() > 1);
    }
}
