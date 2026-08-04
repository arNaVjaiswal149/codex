use crate::legacy_core::config::Config;
use codex_features::Feature;

pub(crate) const TERMINAL_VISUALIZATION_INSTRUCTIONS: &str = "\
- This surface is a terminal. When the formatting rules require a visual, include one in the final answer using compact ASCII diagrams, trees, timelines, or tables.
- Use tables for exact mappings or comparisons rather than collapsing known mappings into prose.
- Use trees for hierarchy or one-to-many relationships, and diagrams or timelines for sequence, change, or state transferred between records across event order.
- Use only ASCII characters in visuals.";

pub(crate) const MERMAID_VISUALIZATION_INSTRUCTIONS: &str = "\
- This terminal renders complete fenced `mermaid` code blocks as diagrams after the final answer finishes.
- When the formatting rules require a visual, prefer one compact Mermaid diagram for relationships, flows, sequences, states, or hierarchies.
- Use `flowchart` for processes and decisions; `sequenceDiagram` for interactions over time; `stateDiagram-v2` for lifecycle transitions; `classDiagram` or `erDiagram` for data structure; `gantt` or `timeline` for schedules; `pie` or `xychart-beta` for quantitative data; `mindmap` or `quadrantChart` for organization and prioritization; and `gitGraph` for version history.
- Keep Mermaid node IDs short, put readable text in quoted labels, and preserve exact names and edge direction.
- Use tables for exact mappings or comparisons rather than collapsing known mappings into prose.
- Use compact ASCII only when Mermaid is not suitable.";

pub(crate) fn with_terminal_visualization_instructions(
    config: &Config,
    control_instructions: Option<String>,
) -> Option<String> {
    let instructions = if crate::mermaid_render::mermaid_rendering_requested(
        config.features.enabled(Feature::MermaidRendering),
    ) {
        MERMAID_VISUALIZATION_INSTRUCTIONS
    } else if config
        .features
        .enabled(Feature::TerminalVisualizationInstructions)
    {
        TERMINAL_VISUALIZATION_INSTRUCTIONS
    } else {
        return control_instructions;
    };

    let existing_instructions =
        control_instructions.or_else(|| config.developer_instructions.clone());
    Some(match existing_instructions.as_deref() {
        Some(existing) if !existing.trim().is_empty() => {
            format!("{existing}\n\n{instructions}")
        }
        _ => instructions.to_string(),
    })
}

#[cfg(test)]
#[path = "terminal_visualization_instructions_tests.rs"]
mod tests;
