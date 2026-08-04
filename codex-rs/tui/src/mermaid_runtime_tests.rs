use super::*;

#[test]
fn cache_key_separates_source_and_theme() {
    assert_ne!(
        cache_key_with_identity("graph TD; A-->B", "dark", "unknown-mmdc", None),
        cache_key_with_identity("graph TD; A-->B", "default", "unknown-mmdc", None)
    );
    assert_ne!(
        cache_key_with_identity("graph TD; A-->B", "dark", "unknown-mmdc", None),
        cache_key_with_identity("graph TD; B-->A", "dark", "unknown-mmdc", None)
    );
}

#[test]
fn all_diagram_types_use_native_display_scale() {
    assert_eq!(DISPLAY_WIDTH_SCALE_HALVES, 2);
}

#[test]
fn mermaid_error_diagnostics_are_not_accepted_as_render_success() {
    assert!(mermaid_output_reports_error(b"Syntax error in text"));
    assert!(mermaid_output_reports_error(b"Parse error on line 3"));
    assert!(!mermaid_output_reports_error(b"render complete"));
}

#[test]
#[ignore = "requires an installed mmdc and browser; exercised by the local integration gate"]
fn real_mmdc_compiles_requested_diagrams_when_runtime_is_installed() {
    let codex_home = find_codex_home().expect("Codex home");
    let mmdc = resolve_mmdc_in(&codex_home).expect("mmdc runtime");
    let temp = tempfile::tempdir().expect("temporary directory");
    let diagrams = [
        (
            "flowchart",
            r#"flowchart TD
  Start --> Decision{Ready?}
  Decision -->|Yes| Done
  Decision -->|No| Start
"#,
        ),
        (
            "sequence",
            r#"sequenceDiagram
  participant A as Client
  participant B as Server
  A->>B: Request
  B-->>A: Response
"#,
        ),
        (
            "state",
            r#"stateDiagram-v2
  [*] --> Idle
  Idle --> Running
  Running --> [*]
"#,
        ),
        (
            "class",
            r#"classDiagram
  class Animal {
    +name: string
    +speak()
  }
  class Dog {
    +fetch()
  }
  Animal <|-- Dog
"#,
        ),
        (
            "entity-relationship",
            r#"erDiagram
  CUSTOMER ||--o{ ORDER : places
  ORDER ||--|{ ITEM : contains
  CUSTOMER {
    int id
    string name
  }
  ORDER {
    int id
    date created
  }
  ITEM {
    int id
    string product
  }
"#,
        ),
        (
            "gantt",
            r#"gantt
  title Project
  dateFormat YYYY-MM-DD
  section Work
  Build :a1, 2026-08-01, 2d
"#,
        ),
        (
            "timeline",
            r#"timeline
  title Small Project
  Monday : Plan
  Tuesday : Build
  Wednesday : Test
  Thursday : Release
"#,
        ),
        (
            "pie",
            r#"pie title Distribution
  "A" : 60
  "B" : 40
"#,
        ),
        (
            "xy-chart",
            r#"xychart-beta
  x-axis [Jan, Feb]
  y-axis "Value" 0 --> 10
  bar [3, 7]
"#,
        ),
        (
            "mindmap",
            r#"mindmap
  root((Root))
    Branch
      Leaf
"#,
        ),
        (
            "quadrant-chart",
            r#"quadrantChart
  title Priority
  x-axis Low --> High
  y-axis Low --> High
  quadrant-1 Do
  Item: [0.8, 0.8]
"#,
        ),
        (
            "git-graph",
            r#"gitGraph
  commit id: "Start"
  branch feature
  checkout feature
  commit id: "Add feature"
  checkout main
  merge feature
  commit id: "Release"
"#,
        ),
    ];

    for (index, (name, source)) in diagrams.into_iter().enumerate() {
        let output = temp.path().join(format!("diagram-{index}.png"));
        compile_mermaid_with_browser(source, &mmdc, resolve_browser().as_deref(), &output, "dark")
            .unwrap_or_else(|error| panic!("mmdc render for {name}: {error}"));
        let image = image::open(output).unwrap_or_else(|error| {
            panic!("read Mermaid PNG for {name}: {error}");
        });
        assert!(image.width() > 1, "{name} width was {}", image.width());
        assert!(image.height() > 1, "{name} height was {}", image.height());
    }
}
