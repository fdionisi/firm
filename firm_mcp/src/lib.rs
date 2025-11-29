//! Firm MCP Server Implementation
//!
//! This crate provides a Model Context Protocol (MCP) server implementation
//! for the Firm business intelligence platform. It exposes Firm's querying
//! capabilities through MCP tools and returns results in TOON format.

use anyhow::Result;
use rmcp::{
    handler::server::{router::tool::ToolRouter, wrapper::Parameters},
    model::*,
    service::RequestContext,
    tool, tool_handler, tool_router, ErrorData as McpError, RoleServer, ServerHandler,
};
use serde_json::json;
use std::sync::Arc;
use tokio::sync::Mutex;

use firm_core::field::FieldValue;
use firm_core::graph::EntityGraph;
use firm_ql::FirmQl;

/// Arguments for the query tool
#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct QueryArgs {
    /// The FirmQL query to execute
    pub query: String,
}

/// The main MCP server for Firm
#[derive(Clone)]
pub struct FirmMcpServer {
    firm_ql: Arc<Mutex<FirmQl>>,
    tool_router: ToolRouter<FirmMcpServer>,
}

#[tool_router]
impl FirmMcpServer {
    /// Creates a new Firm MCP server instance
    pub fn new(graph: EntityGraph) -> Self {
        let firm_ql = FirmQl::new(graph);
        Self {
            firm_ql: Arc::new(Mutex::new(firm_ql)),
            tool_router: Self::tool_router(),
        }
    }

    /// Creates a new Firm MCP server with an empty graph
    pub fn new_empty() -> Self {
        Self::new(EntityGraph::new())
    }

    /// Execute a FirmQL query and return results in TOON format
    #[tool(
        description = "Execute a FirmQL query against the Firm database and return results in TOON format"
    )]
    async fn query(
        &self,
        Parameters(QueryArgs { query }): Parameters<QueryArgs>,
    ) -> Result<CallToolResult, McpError> {
        let firm_ql = self.firm_ql.lock().await;

        match firm_ql.query(&query) {
            Ok(result_set) => {
                let toon_result = self.convert_result_set_to_toon(result_set).map_err(|e| {
                    McpError::internal_error(
                        "toon_conversion_failed",
                        Some(json!({
                            "error": e.to_string(),
                            "query": query
                        })),
                    )
                })?;

                Ok(CallToolResult::success(vec![Content::text(toon_result)]))
            }
            Err(e) => {
                let error_message = format!("Query execution failed: {}", e);
                Ok(CallToolResult::success(vec![Content::text(error_message)]))
            }
        }
    }

    /// Convert a QueryResultSet to TOON format
    fn convert_result_set_to_toon(
        &self,
        result_set: firm_ql::result::QueryResultSet,
    ) -> Result<String> {
        let json_value = self.result_set_to_json_value(result_set)?;

        toon_format::encode_default(&json_value)
            .map_err(|e| anyhow::anyhow!("Failed to encode to TOON: {}", e))
    }

    /// Convert QueryResultSet to JSON Value for TOON encoding
    fn result_set_to_json_value(
        &self,
        result_set: firm_ql::result::QueryResultSet,
    ) -> Result<serde_json::Value> {
        let mut results = Vec::new();

        for row in &result_set.rows {
            let mut row_obj = serde_json::Map::new();

            for (i, column) in result_set.columns.iter().enumerate() {
                let value = row
                    .get(i)
                    .map_err(|e| anyhow::anyhow!("Missing value for column {}: {}", column, e))?;

                let json_value = self.field_value_to_json(value)?;
                row_obj.insert(column.clone(), json_value);
            }

            results.push(serde_json::Value::Object(row_obj));
        }

        Ok(serde_json::Value::Array(results))
    }

    /// Convert a FieldValue to JSON Value
    fn field_value_to_json(&self, value: &FieldValue) -> Result<serde_json::Value> {
        match value {
            FieldValue::String(s) => Ok(serde_json::Value::String(s.clone())),
            FieldValue::Integer(n) => Ok(serde_json::Value::Number(serde_json::Number::from(*n))),
            FieldValue::Float(n) => Ok(serde_json::Value::Number(
                serde_json::Number::from_f64(*n)
                    .ok_or_else(|| anyhow::anyhow!("Invalid float number: {}", n))?,
            )),
            FieldValue::Boolean(b) => Ok(serde_json::Value::Bool(*b)),
            FieldValue::DateTime(date) => Ok(serde_json::Value::String(date.to_string())),
            FieldValue::List(arr) => {
                let json_arr: Result<Vec<_>> =
                    arr.iter().map(|v| self.field_value_to_json(v)).collect();
                Ok(serde_json::Value::Array(json_arr?))
            }
            FieldValue::Reference(ref_val) => Ok(serde_json::Value::String(ref_val.to_string())),
            FieldValue::Currency { amount, currency } => Ok(serde_json::Value::Object({
                let mut map = serde_json::Map::new();
                map.insert(
                    "amount".to_string(),
                    serde_json::Value::String(amount.to_string()),
                );
                map.insert(
                    "currency".to_string(),
                    serde_json::Value::String(currency.to_string()),
                );
                map
            })),
            FieldValue::Path(path) => Ok(serde_json::Value::String(path.display().to_string())),
        }
    }
}

#[tool_handler]
impl ServerHandler for FirmMcpServer {
    fn get_info(&self) -> ServerInfo {
        ServerInfo {
            protocol_version: ProtocolVersion::V_2024_11_05,
            capabilities: ServerCapabilities::builder()
                .enable_tools()
                .enable_resources()
                .build(),
            server_info: Implementation {
                name: "firm-mcp-server".to_string(),
                version: env!("CARGO_PKG_VERSION").to_string(),
                icons: None,
                title: Some("Firm MCP Server".to_string()),
                website_url: None,
            },
            instructions: Some(
                "This server provides access to Firm business intelligence queries. \
                Use the 'query' tool to execute FirmQL queries and receive results in TOON format."
                    .to_string(),
            ),
        }
    }

    async fn list_resources(
        &self,
        _request: Option<PaginatedRequestParam>,
        _: RequestContext<RoleServer>,
    ) -> Result<ListResourcesResult, McpError> {
        Ok(ListResourcesResult {
            resources: vec![
                RawResource::new("firm://query-help", "FirmQL Query Help".to_string())
                    .no_annotation(),
                RawResource::new("firm://schema", "Database Schema".to_string()).no_annotation(),
            ],
            next_cursor: None,
        })
    }

    async fn read_resource(
        &self,
        ReadResourceRequestParam { uri }: ReadResourceRequestParam,
        _: RequestContext<RoleServer>,
    ) -> Result<ReadResourceResult, McpError> {
        match uri.as_str() {
            "firm://query-help" => {
                let help_content = r#"
# FirmQL Query Help

FirmQL is a SQL-like query language for the Firm business intelligence platform.

## Basic Syntax

- `SELECT * FROM entity_type` - Select all records of an entity type
- `SELECT field1, field2 FROM entity_type` - Select specific fields
- `SELECT * FROM entity_type WHERE condition` - Filter records
- `SHOW ENTITY TYPES` - List all available entity types
- `DESCRIBE entity_type` - Show schema for an entity type

## Example Queries

```sql
-- List all entity types
SHOW ENTITY TYPES

-- Describe the schema of a specific entity type
DESCRIBE projects

-- Select all projects
SELECT * FROM projects

-- Select specific fields from projects
SELECT name, status FROM projects WHERE status = 'active'
```

## Data Types

- String: Text values
- Integer: Integer values
- Float: Floating point numbers
- Boolean: true/false values
- DateTime: Date and datetime values
- List: Arrays of values
- Reference: References to other entities
- Currency: Currency amounts with currency code
- Path: File system paths
"#;

                Ok(ReadResourceResult {
                    contents: vec![ResourceContents::text(help_content, uri)],
                })
            }
            "firm://schema" => {
                let firm_ql = self.firm_ql.lock().await;
                match firm_ql.query("SHOW ENTITY TYPES") {
                    Ok(result_set) => {
                        let schema_info =
                            self.convert_result_set_to_toon(result_set).map_err(|e| {
                                McpError::internal_error(
                                    "schema_conversion_failed",
                                    Some(json!({
                                        "error": e.to_string()
                                    })),
                                )
                            })?;

                        Ok(ReadResourceResult {
                            contents: vec![ResourceContents::text(&schema_info, uri)],
                        })
                    }
                    Err(e) => Err(McpError::internal_error(
                        "schema_query_failed",
                        Some(json!({
                            "error": e.to_string()
                        })),
                    )),
                }
            }
            _ => Err(McpError::resource_not_found(
                "resource_not_found",
                Some(json!({ "uri": uri })),
            )),
        }
    }

    async fn list_resource_templates(
        &self,
        _request: Option<PaginatedRequestParam>,
        _: RequestContext<RoleServer>,
    ) -> Result<ListResourceTemplatesResult, McpError> {
        Ok(ListResourceTemplatesResult {
            next_cursor: None,
            resource_templates: Vec::new(),
        })
    }

    async fn initialize(
        &self,
        _request: InitializeRequestParam,
        _context: RequestContext<RoleServer>,
    ) -> Result<InitializeResult, McpError> {
        Ok(self.get_info())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use firm_core::{Entity, EntityId, EntityType, FieldId};

    fn create_test_server() -> FirmMcpServer {
        let mut graph = EntityGraph::new();

        let mut entity = Entity::new(EntityId::new("proj1"), EntityType::new("projects"));
        entity = entity.with_field(
            FieldId::new("name"),
            FieldValue::String("Test Project".to_string()),
        );
        entity = entity.with_field(
            FieldId::new("status"),
            FieldValue::String("active".to_string()),
        );
        entity = entity.with_field(FieldId::new("priority"), FieldValue::Integer(1));
        entity = entity.with_field(FieldId::new("completed"), FieldValue::Boolean(false));

        graph.add_entity(entity).unwrap();
        graph.build();

        FirmMcpServer::new(graph)
    }

    #[tokio::test]
    async fn test_server_creation() {
        let server = FirmMcpServer::new_empty();
        let info = server.get_info();

        assert_eq!(info.server_info.name, "firm-mcp-server");
        assert!(info.capabilities.tools.is_some());
        assert!(info.capabilities.resources.is_some());
    }

    #[tokio::test]
    async fn test_query_tool_exists() {
        let server = create_test_server();
        let tools = server.tool_router.list_all();

        assert!(tools.iter().any(|tool| tool.name == "query"));
    }

    // Note: Integration tests for resources would require proper MCP context setup
    // For now, we focus on unit tests for the conversion functions

    #[test]
    fn test_field_value_to_json_conversion() {
        let server = FirmMcpServer::new_empty();

        let text_value = FieldValue::String("hello".to_string());
        let json_result = server.field_value_to_json(&text_value).unwrap();
        assert_eq!(json_result, serde_json::Value::String("hello".to_string()));

        let number_value = FieldValue::Integer(42);
        let json_result = server.field_value_to_json(&number_value).unwrap();
        assert_eq!(
            json_result,
            serde_json::Value::Number(serde_json::Number::from(42))
        );

        let float_value = FieldValue::Float(42.5);
        let json_result = server.field_value_to_json(&float_value).unwrap();
        assert_eq!(
            json_result,
            serde_json::Value::Number(serde_json::Number::from_f64(42.5).unwrap())
        );

        let bool_value = FieldValue::Boolean(true);
        let json_result = server.field_value_to_json(&bool_value).unwrap();
        assert_eq!(json_result, serde_json::Value::Bool(true));
    }
}
