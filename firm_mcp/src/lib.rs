//! Firm MCP Server Implementation
//!
//! This crate provides a Model Context Protocol (MCP) server implementation
//! for the Firm business intelligence platform. It exposes Firm's querying
//! capabilities through MCP tools and returns results in TOON format.

use anyhow::Result;
use rmcp::{model::*, service::RequestContext, ErrorData as McpError, RoleServer, ServerHandler};
use serde_json::json;
use std::sync::Arc;
use tokio::sync::Mutex;

use firm_core::field::FieldValue;
use firm_core::graph::EntityGraph;
use firm_ql::FirmQl;

/// Configuration for customizing MCP server interface
#[derive(Debug, Clone)]
pub struct FirmMcpConfig {
    /// Prefix for tool names (e.g., "workspace-a" -> "workspace-a-query")
    pub tool_prefix: Option<String>,
    /// Prefix for resource paths (e.g., "workspace-a" -> "firm://workspace-a/schema")
    pub resource_prefix: Option<String>,
    /// Server name override
    pub server_name: Option<String>,
    /// Tool description override
    pub tool_description: Option<String>,
}

impl Default for FirmMcpConfig {
    fn default() -> Self {
        Self {
            tool_prefix: None,
            resource_prefix: None,
            server_name: None,
            tool_description: None,
        }
    }
}

impl FirmMcpConfig {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_prefix(mut self, prefix: String) -> Self {
        self.tool_prefix = Some(prefix.clone());
        self.resource_prefix = Some(prefix);
        self
    }

    pub fn with_tool_description(mut self, description: String) -> Self {
        self.tool_description = Some(description);
        self
    }

    pub fn with_tool_prefix(mut self, prefix: String) -> Self {
        self.tool_prefix = Some(prefix);
        self
    }

    pub fn with_resource_prefix(mut self, prefix: String) -> Self {
        self.resource_prefix = Some(prefix);
        self
    }

    pub fn with_server_name(mut self, name: String) -> Self {
        self.server_name = Some(name);
        self
    }
}

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
    config: FirmMcpConfig,
}

impl FirmMcpServer {
    /// Creates a new Firm MCP server instance with default configuration
    pub fn new(graph: EntityGraph) -> Self {
        Self::with_config(graph, FirmMcpConfig::default())
    }

    /// Creates a new Firm MCP server instance with custom configuration
    pub fn with_config(graph: EntityGraph, config: FirmMcpConfig) -> Self {
        let firm_ql = FirmQl::new(graph);
        Self {
            firm_ql: Arc::new(Mutex::new(firm_ql)),
            config,
        }
    }

    /// Creates a new Firm MCP server with an empty graph
    pub fn new_empty() -> Self {
        Self::new(EntityGraph::new())
    }

    /// Get the actual tool name with prefix applied
    fn get_tool_name(&self) -> String {
        match &self.config.tool_prefix {
            Some(prefix) => format!("{}-query", prefix),
            None => "query".to_string(),
        }
    }

    /// Execute a FirmQL query and return results in TOON format - internal implementation
    async fn execute_query_internal(&self, query: String) -> Result<CallToolResult, McpError> {
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

impl ServerHandler for FirmMcpServer {
    async fn list_tools(
        &self,
        _request: Option<PaginatedRequestParam>,
        _: RequestContext<RoleServer>,
    ) -> Result<ListToolsResult, McpError> {
        let tool_name = self.get_tool_name();
        let tools = vec![Tool {
            name: tool_name.into(),
            description: Some(
                self.config
                    .tool_description
                    .clone()
                    .unwrap_or_else(|| "Execute a FirmQL query against the Firm database and return results in TOON format".to_string())
                    .into()
            ),
            input_schema: {
                let mut schema = serde_json::Map::new();
                schema.insert("type".to_string(), json!("object"));
                let mut properties = serde_json::Map::new();
                let mut query_prop = serde_json::Map::new();
                query_prop.insert("type".to_string(), json!("string"));
                query_prop.insert("description".to_string(), json!("The FirmQL query to execute"));
                properties.insert("query".to_string(), json!(query_prop));
                schema.insert("properties".to_string(), json!(properties));
                schema.insert("required".to_string(), json!(["query"]));
                std::sync::Arc::new(schema)
            },
            annotations: None,
            icons: None,
            meta: None,
            title: None,
            output_schema: None,
        }];

        Ok(ListToolsResult {
            tools,
            next_cursor: None,
        })
    }

    async fn call_tool(
        &self,
        CallToolRequestParam { name, arguments }: CallToolRequestParam,
        _: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, McpError> {
        let expected_tool_name = self.get_tool_name();

        if name != expected_tool_name {
            return Ok(CallToolResult::success(vec![Content::text(format!(
                "Unknown tool: {}. Available tools: {}",
                name, expected_tool_name
            ))]));
        }

        let args_value = arguments.unwrap_or_default();
        let query_args: QueryArgs = serde_json::from_value(json!(args_value)).map_err(|e| {
            McpError::invalid_params(
                "invalid_arguments",
                Some(serde_json::json!({
                    "error": e.to_string()
                })),
            )
        })?;

        self.execute_query_internal(query_args.query).await
    }
    fn get_info(&self) -> ServerInfo {
        ServerInfo {
            protocol_version: ProtocolVersion::V_2024_11_05,
            capabilities: ServerCapabilities::builder()
                .enable_tools()
                .enable_resources()
                .build(),
            server_info: Implementation {
                name: self
                    .config
                    .server_name
                    .clone()
                    .unwrap_or_else(|| "firm-mcp-server".to_string()),
                version: env!("CARGO_PKG_VERSION").to_string(),
                icons: None,
                title: Some(
                    self.config
                        .server_name
                        .clone()
                        .unwrap_or_else(|| "Firm MCP Server".to_string()),
                ),
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
        let query_help_uri = match &self.config.resource_prefix {
            Some(prefix) => format!("firm://{}/query-help", prefix),
            None => "firm://query-help".to_string(),
        };

        let schema_uri = match &self.config.resource_prefix {
            Some(prefix) => format!("firm://{}/schema", prefix),
            None => "firm://schema".to_string(),
        };

        Ok(ListResourcesResult {
            resources: vec![
                RawResource::new(&query_help_uri, "FirmQL Query Help".to_string()).no_annotation(),
                RawResource::new(&schema_uri, "Database Schema".to_string()).no_annotation(),
            ],
            next_cursor: None,
        })
    }

    async fn read_resource(
        &self,
        ReadResourceRequestParam { uri }: ReadResourceRequestParam,
        _: RequestContext<RoleServer>,
    ) -> Result<ReadResourceResult, McpError> {
        let query_help_uri = match &self.config.resource_prefix {
            Some(prefix) => format!("firm://{}/query-help", prefix),
            None => "firm://query-help".to_string(),
        };

        let schema_uri = match &self.config.resource_prefix {
            Some(prefix) => format!("firm://{}/schema", prefix),
            None => "firm://schema".to_string(),
        };

        match uri.as_str() {
            uri if uri == query_help_uri => {
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
            uri if uri == schema_uri => {
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
        let expected_tool_name = server.get_tool_name();

        assert_eq!(expected_tool_name, "query");

        // Test with custom prefix and description
        let config = FirmMcpConfig::new()
            .with_tool_prefix("test-workspace".to_string())
            .with_tool_description("Query the test workspace database".to_string());
        let server_with_prefix = FirmMcpServer::with_config(EntityGraph::new(), config);
        let prefixed_tool_name = server_with_prefix.get_tool_name();
        assert_eq!(prefixed_tool_name, "test-workspace-query");
    }

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
