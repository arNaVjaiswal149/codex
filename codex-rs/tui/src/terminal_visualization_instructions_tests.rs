use super::*;
use crate::legacy_core::config::ConfigBuilder;
use pretty_assertions::assert_eq;

#[test]
fn terminal_visualization_instructions_list_every_supported_mermaid_kind() {
    for kind in [
        "flowchart",
        "sequenceDiagram",
        "stateDiagram-v2",
        "classDiagram",
        "erDiagram",
        "gantt",
        "timeline",
        "pie",
        "xychart-beta",
        "mindmap",
        "quadrantChart",
        "gitGraph",
    ] {
        assert!(
            MERMAID_VISUALIZATION_INSTRUCTIONS.contains(kind),
            "missing Mermaid kind {kind} from terminal visualization instructions"
        );
    }
}

#[tokio::test]
async fn mermaid_feature_gates_model_instructions() {
    let codex_home = tempfile::tempdir().expect("temporary Codex home");
    let mut config = ConfigBuilder::default()
        .codex_home(codex_home.path().to_path_buf())
        .build()
        .await
        .expect("build test config");
    assert!(!config.features.enabled(Feature::MermaidRendering));

    let control = Some("Developer override.".to_string());
    assert_eq!(
        with_terminal_visualization_instructions(&config, control.clone()),
        control,
    );

    config
        .features
        .enable(Feature::MermaidRendering)
        .expect("enable Mermaid rendering");
    assert_eq!(
        with_terminal_visualization_instructions(&config, Some("Developer override.".to_string()),),
        Some(format!(
            "Developer override.\n\n{MERMAID_VISUALIZATION_INSTRUCTIONS}"
        )),
    );
}

#[tokio::test]
async fn legacy_feature_keeps_ascii_instructions() {
    let codex_home = tempfile::tempdir().expect("temporary Codex home");
    let mut config = ConfigBuilder::default()
        .codex_home(codex_home.path().to_path_buf())
        .build()
        .await
        .expect("build test config");
    config
        .features
        .enable(Feature::TerminalVisualizationInstructions)
        .expect("enable terminal visualization instructions");

    assert_eq!(
        with_terminal_visualization_instructions(&config, None),
        Some(TERMINAL_VISUALIZATION_INSTRUCTIONS.to_string()),
    );
}
