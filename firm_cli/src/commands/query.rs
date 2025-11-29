use std::path::PathBuf;

use firm_core::graph::EntityGraph;
use firm_ql::FirmQl;

use crate::commands::{build_workspace, load_workspace_files};
use crate::errors::CliError;
use crate::files::load_current_graph;
use crate::ui::{self, OutputFormat};

/// Execute a SQL-like query against the workspace.
pub fn execute_query(
    workspace_path: &PathBuf,
    query: String,
    output_format: OutputFormat,
) -> Result<(), CliError> {
    let graph = load_entity_graph(workspace_path)?;

    let firm_ql = FirmQl::new(graph);

    match firm_ql.query(&query) {
        Ok(result_set) => {
            match output_format {
                OutputFormat::Pretty => print_table_format(&result_set),
                OutputFormat::Json => print_json_format(&result_set)?,
            }
            Ok(())
        }
        Err(e) => {
            ui::error_with_details("Query execution failed", &e.to_string());
            Err(CliError::QueryError)
        }
    }
}

/// Load the entity graph from cache or build it fresh.
fn load_entity_graph(workspace_path: &PathBuf) -> Result<EntityGraph, CliError> {
    match load_current_graph(workspace_path) {
        Ok(graph) => Ok(graph),
        Err(_) => {
            ui::debug("Graph cache not found or invalid, building fresh graph");
            let mut workspace = firm_lang::workspace::Workspace::new();
            load_workspace_files(workspace_path, &mut workspace)
                .map_err(|_| CliError::BuildError)?;
            let build = build_workspace(workspace).map_err(|_| CliError::BuildError)?;

            let mut graph = EntityGraph::new();
            graph
                .add_entities(build.entities)
                .map_err(|_| CliError::BuildError)?;
            graph.build();
            Ok(graph)
        }
    }
}

/// Print results in table format.
fn print_table_format(result_set: &firm_ql::result::QueryResultSet) {
    if result_set.rows.is_empty() {
        println!("No results found.");
        return;
    }

    let header = result_set.columns.join(" | ");
    println!("{}", header);
    println!("{}", "-".repeat(header.len()));

    for row in &result_set.rows {
        let row_str: Vec<String> = row.values.iter().map(|v| format!("{}", v)).collect();
        println!("{}", row_str.join(" | "));
    }

    println!();
    println!(
        "({} row{})",
        result_set.len(),
        if result_set.len() == 1 { "" } else { "s" }
    );

    if let Some(time_ms) = result_set.metadata.execution_time_ms {
        println!("Execution time: {}ms", time_ms);
    }
    if let Some(entities_scanned) = result_set.metadata.entities_scanned {
        println!("Entities scanned: {}", entities_scanned);
    }
}

/// Print results in JSON format.
fn print_json_format(result_set: &firm_ql::result::QueryResultSet) -> Result<(), CliError> {
    let json = serde_json::to_string_pretty(result_set).map_err(|_| CliError::QueryError)?;
    println!("{}", json);
    Ok(())
}
