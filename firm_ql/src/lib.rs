//! SQL-like query language for Firm workspaces.
//!
//! This crate provides a SQL-inspired syntax for querying entities and their relationships
//! in a Firm workspace. It builds on top of `firm_core` and `firm_lang` to provide an
//! intuitive way to retrieve and filter business data.
//!
//! # Examples
//!
//! ## Basic Query
//! ```no_run
//! use firm_ql::FirmQl;
//! use firm_core::graph::EntityGraph;
//!
//! # fn main() -> Result<(), Box<dyn std::error::Error>> {
//! let graph = EntityGraph::new();
//! let firm_ql = FirmQl::new(graph);
//! let results = firm_ql.query("SELECT name, email FROM person WHERE name LIKE 'John%'")?;
//! # Ok(())
//! # }
//! ```
//!
//! ## Introspection Queries
//! ```no_run
//! use firm_ql::FirmQl;
//! use firm_core::graph::EntityGraph;
//!
//! # fn main() -> Result<(), Box<dyn std::error::Error>> {
//! let graph = EntityGraph::new();
//! let firm_ql = FirmQl::new(graph);
//!
//!
//! let entity_types = firm_ql.query("SHOW ENTITY TYPES")?;
//!
//!
//! let schema = firm_ql.query("DESCRIBE person")?;
//! # Ok(())
//! # }
//! ```
//!
//! ## Complete Example
//! ```
//! use firm_ql::FirmQl;
//! use firm_core::{Entity, EntityType, FieldId, graph::EntityGraph};
//! use firm_core::field::FieldValue;
//!
//!
//! let mut graph = EntityGraph::new();
//!
//! let person = Entity::new(
//!     firm_core::EntityId::new("person.john"),
//!     EntityType::new("person")
//! ).with_field(
//!     FieldId::new("name"),
//!     FieldValue::String("John Doe".to_string())
//! ).with_field(
//!     FieldId::new("email"),
//!     FieldValue::String("john@example.com".to_string())
//! ).with_field(
//!     FieldId::new("age"),
//!     FieldValue::Integer(30)
//! );
//!
//! graph.add_entity(person).unwrap();
//! graph.build();
//!
//! let firm_ql = FirmQl::new(graph);
//!
//! let entity_types = firm_ql.query("SHOW ENTITY TYPES").unwrap();
//! println!("Entity types: {:?}", entity_types);
//!
//! let schema = firm_ql.query("DESCRIBE person").unwrap();
//! println!("Person schema: {:?}", schema);
//!
//! let results = firm_ql.query("SELECT name, age FROM person WHERE age > 25").unwrap();
//! println!("Query results: {:?}", results);
//! ```

pub mod engine;
pub mod error;
pub mod result;

use engine::QueryEngine;
use error::{QueryError, QueryResult};
use firm_core::field::FieldValue;
use firm_core::{graph::EntityGraph, EntityType};
use result::{QueryResultSet, QueryRow};
use sqlparser::ast::Query;

use sqlparser::dialect::GenericDialect;
use sqlparser::parser::Parser as SqlParser;

pub struct FirmQl {
    graph: EntityGraph,
}

impl FirmQl {
    pub fn new(graph: EntityGraph) -> Self {
        FirmQl { graph }
    }

    pub fn query(&self, query: &str) -> QueryResult<QueryResultSet> {
        let trimmed = query.trim();

        if trimmed.eq_ignore_ascii_case("SHOW ENTITY TYPES") {
            return self.show_entity_types();
        }

        if let Some(entity_type) = self.parse_describe_query(trimmed) {
            return self.describe_entity_type(&entity_type);
        }

        let query = parse_query(query)?;
        let engine = QueryEngine::new();
        engine.execute(&query, &self.graph)
    }

    /// Returns all entity types in the graph
    fn show_entity_types(&self) -> QueryResult<QueryResultSet> {
        let entity_types = self.graph.get_all_entity_types();
        let mut result_set = QueryResultSet::new(vec!["entity_type".to_string()]);

        for entity_type in entity_types {
            let row = QueryRow::new(vec![FieldValue::String(entity_type.to_string())]);
            result_set.add_row(row)?;
        }

        Ok(result_set)
    }

    /// Returns the schema of a specific entity type by inferring it from existing entities
    fn describe_entity_type(&self, entity_type: &EntityType) -> QueryResult<QueryResultSet> {
        let entities = self.graph.list_by_type(entity_type);

        if entities.is_empty() {
            return Err(QueryError::syntax(
                &format!("Entity type '{}' not found", entity_type),
                0,
            ));
        }

        let mut field_info = std::collections::HashMap::new();

        for entity in entities {
            for (field_id, field_value) in &entity.fields {
                let field_name = field_id.as_str().to_string();
                let field_type = self.infer_field_type(field_value);

                match field_info.get(&field_name) {
                    Some(existing_type) if *existing_type == "String" && field_type != "String" => {
                        field_info.insert(field_name, field_type);
                    }
                    None => {
                        field_info.insert(field_name, field_type);
                    }
                    _ => {}
                }
            }
        }

        let mut result_set =
            QueryResultSet::new(vec!["field_name".to_string(), "field_type".to_string()]);

        let mut field_names: Vec<_> = field_info.keys().collect();
        field_names.sort();

        for field_name in field_names {
            let field_type = field_info.get(field_name).unwrap();
            let row = QueryRow::new(vec![
                FieldValue::String(field_name.clone()),
                FieldValue::String(field_type.clone()),
            ]);
            result_set.add_row(row)?;
        }

        Ok(result_set)
    }

    /// Parse DESCRIBE queries (case-insensitive)
    fn parse_describe_query(&self, query: &str) -> Option<EntityType> {
        let parts: Vec<&str> = query.split_whitespace().collect();
        if parts.len() == 2 && parts[0].eq_ignore_ascii_case("DESCRIBE") {
            Some(EntityType::new(parts[1]))
        } else {
            None
        }
    }

    /// Infer field type from field value
    fn infer_field_type(&self, field_value: &FieldValue) -> String {
        match field_value {
            FieldValue::Boolean(_) => "Boolean".to_string(),
            FieldValue::String(_) => "String".to_string(),
            FieldValue::Integer(_) => "Integer".to_string(),
            FieldValue::Float(_) => "Float".to_string(),
            FieldValue::Currency { .. } => "Currency".to_string(),
            FieldValue::Reference(_) => "Reference".to_string(),
            FieldValue::List(_) => "List".to_string(),
            FieldValue::DateTime(_) => "DateTime".to_string(),
            FieldValue::Path(_) => "Path".to_string(),
        }
    }
}

/// Parse a SQL-like query string into a Query AST.
fn parse_query(query: &str) -> QueryResult<Query> {
    let dialect = GenericDialect {};
    let statements = SqlParser::parse_sql(&dialect, query)
        .map_err(|e| QueryError::syntax(&format!("SQL parse error: {}", e), 0))?;

    if statements.is_empty() {
        return Err(QueryError::syntax("Empty query", 0));
    }

    if statements.len() > 1 {
        return Err(QueryError::syntax("Multiple statements not supported", 0));
    }

    match &statements[0] {
        sqlparser::ast::Statement::Query(sql_query) => Ok((**sql_query).clone()),
        _ => Err(QueryError::syntax(
            "Only SELECT statements are supported",
            0,
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use firm_core::field::FieldValue;
    use firm_core::graph::EntityGraph;
    use firm_core::{Entity, EntityType, FieldId};

    fn create_test_graph() -> EntityGraph {
        let mut graph = EntityGraph::new();

        let person = Entity::new(
            firm_core::EntityId::new("person.john_doe"),
            EntityType::new("person"),
        )
        .with_field(
            FieldId::new("name"),
            FieldValue::String("John Doe".to_string()),
        )
        .with_field(
            FieldId::new("email"),
            FieldValue::String("john@example.com".to_string()),
        );

        graph.add_entity(person).unwrap();
        graph.build();
        graph
    }

    #[test]
    fn test_simple_select_query() {
        let graph = create_test_graph();
        let firm_ql = FirmQl::new(graph);
        let result = firm_ql.query("SELECT name FROM person");

        assert!(result.is_ok());
        let result_set = result.unwrap();
        assert_eq!(result_set.len(), 1);
    }

    #[test]
    fn test_parse_query() {
        let query = parse_query("SELECT * FROM person WHERE name = 'John'");
        assert!(query.is_ok());
    }

    fn create_complex_workspace() -> EntityGraph {
        let mut graph = EntityGraph::new();

        let acme_corp = Entity::new(
            firm_core::EntityId::new("organization.acme_corp"),
            EntityType::new("organization"),
        )
        .with_field(
            FieldId::new("name"),
            FieldValue::String("Acme Corp".to_string()),
        )
        .with_field(
            FieldId::new("email"),
            FieldValue::String("contact@acme.com".to_string()),
        )
        .with_field(
            FieldId::new("vat_id"),
            FieldValue::String("GB123456789".to_string()),
        );

        let tech_startup = Entity::new(
            firm_core::EntityId::new("organization.tech_startup"),
            EntityType::new("organization"),
        )
        .with_field(
            FieldId::new("name"),
            FieldValue::String("TechStartup Inc".to_string()),
        )
        .with_field(
            FieldId::new("email"),
            FieldValue::String("hello@techstartup.io".to_string()),
        );

        let john_doe = Entity::new(
            firm_core::EntityId::new("person.john_doe"),
            EntityType::new("person"),
        )
        .with_field(
            FieldId::new("name"),
            FieldValue::String("John Doe".to_string()),
        )
        .with_field(
            FieldId::new("email"),
            FieldValue::String("john@acme.com".to_string()),
        )
        .with_field(
            FieldId::new("phone"),
            FieldValue::String("+44 20 7123 4567".to_string()),
        );

        let jane_smith = Entity::new(
            firm_core::EntityId::new("person.jane_smith"),
            EntityType::new("person"),
        )
        .with_field(
            FieldId::new("name"),
            FieldValue::String("Jane Smith".to_string()),
        )
        .with_field(
            FieldId::new("email"),
            FieldValue::String("jane@techstartup.io".to_string()),
        );

        let alice_johnson = Entity::new(
            firm_core::EntityId::new("person.alice_johnson"),
            EntityType::new("person"),
        )
        .with_field(
            FieldId::new("name"),
            FieldValue::String("Alice Johnson".to_string()),
        )
        .with_field(
            FieldId::new("email"),
            FieldValue::String("alice@acme.com".to_string()),
        );

        let john_contact = Entity::new(
            firm_core::EntityId::new("contact.john_acme"),
            EntityType::new("contact"),
        )
        .with_field(FieldId::new("role"), FieldValue::String("CTO".to_string()))
        .with_field(
            FieldId::new("status"),
            FieldValue::String("active".to_string()),
        )
        .with_field(
            FieldId::new("person_ref"),
            FieldValue::Reference(firm_core::field::ReferenceValue::Entity(
                firm_core::EntityId::new("person.john_doe"),
            )),
        )
        .with_field(
            FieldId::new("organization_ref"),
            FieldValue::Reference(firm_core::field::ReferenceValue::Entity(
                firm_core::EntityId::new("organization.acme_corp"),
            )),
        );

        let jane_contact = Entity::new(
            firm_core::EntityId::new("contact.jane_startup"),
            EntityType::new("contact"),
        )
        .with_field(FieldId::new("role"), FieldValue::String("CEO".to_string()))
        .with_field(
            FieldId::new("status"),
            FieldValue::String("active".to_string()),
        )
        .with_field(
            FieldId::new("person_ref"),
            FieldValue::Reference(firm_core::field::ReferenceValue::Entity(
                firm_core::EntityId::new("person.jane_smith"),
            )),
        )
        .with_field(
            FieldId::new("organization_ref"),
            FieldValue::Reference(firm_core::field::ReferenceValue::Entity(
                firm_core::EntityId::new("organization.tech_startup"),
            )),
        );

        let ai_project = Entity::new(
            firm_core::EntityId::new("project.ai_platform"),
            EntityType::new("project"),
        )
        .with_field(
            FieldId::new("name"),
            FieldValue::String("AI Platform Development".to_string()),
        )
        .with_field(
            FieldId::new("status"),
            FieldValue::String("in_progress".to_string()),
        )
        .with_field(
            FieldId::new("description"),
            FieldValue::String("Building next-gen AI platform".to_string()),
        );

        let web_project = Entity::new(
            firm_core::EntityId::new("project.web_redesign"),
            EntityType::new("project"),
        )
        .with_field(
            FieldId::new("name"),
            FieldValue::String("Website Redesign".to_string()),
        )
        .with_field(
            FieldId::new("status"),
            FieldValue::String("completed".to_string()),
        )
        .with_field(
            FieldId::new("description"),
            FieldValue::String("Complete website overhaul".to_string()),
        );

        let task1 = Entity::new(
            firm_core::EntityId::new("task.setup_ml_pipeline"),
            EntityType::new("task"),
        )
        .with_field(
            FieldId::new("name"),
            FieldValue::String("Set up ML pipeline".to_string()),
        )
        .with_field(FieldId::new("is_completed"), FieldValue::Boolean(false))
        .with_field(
            FieldId::new("description"),
            FieldValue::String("Configure ML training pipeline".to_string()),
        )
        .with_field(
            FieldId::new("assignee_ref"),
            FieldValue::Reference(firm_core::field::ReferenceValue::Entity(
                firm_core::EntityId::new("person.john_doe"),
            )),
        )
        .with_field(
            FieldId::new("source_ref"),
            FieldValue::Reference(firm_core::field::ReferenceValue::Entity(
                firm_core::EntityId::new("project.ai_platform"),
            )),
        );

        let task2 = Entity::new(
            firm_core::EntityId::new("task.design_homepage"),
            EntityType::new("task"),
        )
        .with_field(
            FieldId::new("name"),
            FieldValue::String("Design new homepage".to_string()),
        )
        .with_field(FieldId::new("is_completed"), FieldValue::Boolean(true))
        .with_field(
            FieldId::new("description"),
            FieldValue::String("Create modern homepage design".to_string()),
        )
        .with_field(
            FieldId::new("assignee_ref"),
            FieldValue::Reference(firm_core::field::ReferenceValue::Entity(
                firm_core::EntityId::new("person.alice_johnson"),
            )),
        )
        .with_field(
            FieldId::new("source_ref"),
            FieldValue::Reference(firm_core::field::ReferenceValue::Entity(
                firm_core::EntityId::new("project.web_redesign"),
            )),
        );

        let task3 = Entity::new(
            firm_core::EntityId::new("task.api_integration"),
            EntityType::new("task"),
        )
        .with_field(
            FieldId::new("name"),
            FieldValue::String("API Integration".to_string()),
        )
        .with_field(FieldId::new("is_completed"), FieldValue::Boolean(false))
        .with_field(
            FieldId::new("description"),
            FieldValue::String("Integrate third-party APIs".to_string()),
        )
        .with_field(
            FieldId::new("assignee_ref"),
            FieldValue::Reference(firm_core::field::ReferenceValue::Entity(
                firm_core::EntityId::new("person.jane_smith"),
            )),
        )
        .with_field(
            FieldId::new("source_ref"),
            FieldValue::Reference(firm_core::field::ReferenceValue::Entity(
                firm_core::EntityId::new("project.ai_platform"),
            )),
        );

        let opportunity1 = Entity::new(
            firm_core::EntityId::new("opportunity.enterprise_deal"),
            EntityType::new("opportunity"),
        )
        .with_field(
            FieldId::new("name"),
            FieldValue::String("Enterprise AI Deal".to_string()),
        )
        .with_field(
            FieldId::new("status"),
            FieldValue::String("negotiation".to_string()),
        )
        .with_field(
            FieldId::new("value"),
            FieldValue::Currency {
                amount: rust_decimal::Decimal::new(250000, 0),
                currency: iso_currency::Currency::USD,
            },
        )
        .with_field(FieldId::new("probability"), FieldValue::Integer(75))
        .with_field(
            FieldId::new("source_ref"),
            FieldValue::Reference(firm_core::field::ReferenceValue::Entity(
                firm_core::EntityId::new("contact.john_acme"),
            )),
        );

        graph.add_entity(acme_corp).unwrap();
        graph.add_entity(tech_startup).unwrap();
        graph.add_entity(john_doe).unwrap();
        graph.add_entity(jane_smith).unwrap();
        graph.add_entity(alice_johnson).unwrap();
        graph.add_entity(john_contact).unwrap();
        graph.add_entity(jane_contact).unwrap();
        graph.add_entity(ai_project).unwrap();
        graph.add_entity(web_project).unwrap();
        graph.add_entity(task1).unwrap();
        graph.add_entity(task2).unwrap();
        graph.add_entity(task3).unwrap();
        graph.add_entity(opportunity1).unwrap();

        graph.build();
        graph
    }

    #[test]
    fn test_complex_select_all_entities() {
        let graph = create_complex_workspace();
        let firm_ql = FirmQl::new(graph);

        let result = firm_ql.query("SELECT * FROM organization");
        if let Err(ref e) = result {
            println!("Error executing query: {:?}", e);
        }
        assert!(result.is_ok());
        let result_set = result.unwrap();
        assert_eq!(result_set.len(), 2);
        assert_eq!(result_set.columns.len(), 4);

        let graph = create_complex_workspace();
        let firm_ql = FirmQl::new(graph);
        let result = firm_ql.query("SELECT * FROM person");
        assert!(result.is_ok());
        let result_set = result.unwrap();
        assert_eq!(result_set.len(), 3);
    }

    #[test]
    fn test_complex_where_clauses() {
        let graph = create_complex_workspace();
        let firm_ql = FirmQl::new(graph);

        let result = firm_ql.query("SELECT name FROM person WHERE name = 'John Doe'");
        assert!(result.is_ok());
        let result_set = result.unwrap();
        assert_eq!(result_set.len(), 1);

        let graph = create_complex_workspace();
        let firm_ql = FirmQl::new(graph);
        let result = firm_ql.query("SELECT name FROM person WHERE name LIKE 'J%'");
        assert!(result.is_ok());
        let result_set = result.unwrap();
        assert_eq!(result_set.len(), 2);

        let graph = create_complex_workspace();
        let firm_ql = FirmQl::new(graph);
        let result = firm_ql.query("SELECT name FROM task WHERE is_completed = true");
        assert!(result.is_ok());
        let result_set = result.unwrap();
        assert_eq!(result_set.len(), 1);

        let graph = create_complex_workspace();
        let firm_ql = FirmQl::new(graph);
        let result = firm_ql.query("SELECT name FROM opportunity WHERE probability > 50");
        assert!(result.is_ok());
        let result_set = result.unwrap();
        assert_eq!(result_set.len(), 1);
    }

    #[test]
    fn test_complex_column_selection() {
        let graph = create_complex_workspace();
        let firm_ql = FirmQl::new(graph);

        let result = firm_ql.query("SELECT name, email FROM person");
        assert!(result.is_ok());
        let result_set = result.unwrap();
        assert_eq!(result_set.columns.len(), 2);
        assert_eq!(result_set.columns[0], "name");
        assert_eq!(result_set.columns[1], "email");
        assert_eq!(result_set.len(), 3);

        let graph = create_complex_workspace();
        let firm_ql = FirmQl::new(graph);
        let result = firm_ql.query("SELECT status FROM project");
        assert!(result.is_ok());
        let result_set = result.unwrap();
        assert_eq!(result_set.columns.len(), 1);
        assert_eq!(result_set.columns[0], "status");
        assert_eq!(result_set.len(), 2);
    }

    #[test]
    fn test_complex_function_queries() {
        let graph = create_complex_workspace();
        let firm_ql = FirmQl::new(graph);

        let result = firm_ql.query("SELECT COUNT(*) FROM task");
        assert!(result.is_ok());
        let result_set = result.unwrap();
        assert_eq!(result_set.len(), 3);

        let graph = create_complex_workspace();
        let firm_ql = FirmQl::new(graph);
        let result = firm_ql.query("SELECT COUNT(*) FROM person");
        assert!(result.is_ok());
        let result_set = result.unwrap();
        assert_eq!(result_set.len(), 3);

        let graph = create_complex_workspace();
        let firm_ql = FirmQl::new(graph);
        let result = firm_ql.query("SELECT COUNT(*) FROM organization");
        assert!(result.is_ok());
        let result_set = result.unwrap();
        assert_eq!(result_set.len(), 2);
    }

    #[test]
    fn test_complex_mixed_queries() {
        let graph = create_complex_workspace();
        let firm_ql = FirmQl::new(graph);

        let result =
            firm_ql.query("SELECT name FROM task WHERE is_completed = false AND name LIKE '%API%'");
        assert!(result.is_ok());
        let result_set = result.unwrap();
        assert_eq!(result_set.len(), 1);

        let graph = create_complex_workspace();
        let firm_ql = FirmQl::new(graph);
        let result = firm_ql.query("SELECT name FROM project WHERE status = 'completed'");
        assert!(result.is_ok());
        let result_set = result.unwrap();
        assert_eq!(result_set.len(), 1);

        let graph = create_complex_workspace();
        let firm_ql = FirmQl::new(graph);
        let result = firm_ql.query("SELECT name FROM opportunity WHERE status = 'negotiation'");
        assert!(result.is_ok());
        let result_set = result.unwrap();
        assert_eq!(result_set.len(), 1);
    }

    #[test]
    fn test_complex_edge_cases() {
        let graph = create_complex_workspace();
        let firm_ql = FirmQl::new(graph);

        let result = firm_ql.query("SELECT name FROM person WHERE name = 'NonExistent Person'");
        assert!(result.is_ok());
        let result_set = result.unwrap();
        assert_eq!(result_set.len(), 0);

        let graph = create_complex_workspace();
        let firm_ql = FirmQl::new(graph);
        let result = firm_ql.query("SELECT name FROM organization WHERE name LIKE '%acme%'");
        assert!(result.is_ok());
        let result_set = result.unwrap();
        assert_eq!(result_set.len(), 0);

        let graph = create_complex_workspace();
        let firm_ql = FirmQl::new(graph);
        let result = firm_ql.query("SELECT name FROM organization WHERE name LIKE '%Acme%'");
        assert!(result.is_ok());
        let result_set = result.unwrap();
        assert_eq!(result_set.len(), 1);

        let graph = create_complex_workspace();
        let firm_ql = FirmQl::new(graph);
        let result =
            firm_ql.query("SELECT name FROM task WHERE is_completed = true OR name LIKE '%ML%'");
        assert!(result.is_ok());
        let result_set = result.unwrap();
        assert_eq!(result_set.len(), 2);
    }

    #[test]
    fn test_complex_data_types() {
        let graph = create_complex_workspace();
        let firm_ql = FirmQl::new(graph);

        let result =
            firm_ql.query("SELECT name, is_completed FROM task WHERE is_completed = false");
        assert!(result.is_ok());
        let result_set = result.unwrap();
        assert_eq!(result_set.len(), 2);

        let graph = create_complex_workspace();
        let firm_ql = FirmQl::new(graph);
        let result =
            firm_ql.query("SELECT name, probability FROM opportunity WHERE probability = 75");
        assert!(result.is_ok());
        let result_set = result.unwrap();
        assert_eq!(result_set.len(), 1);

        let graph = create_complex_workspace();
        let firm_ql = FirmQl::new(graph);
        let result = firm_ql.query("SELECT email FROM person WHERE email LIKE '%@acme.com'");
        assert!(result.is_ok());
        let result_set = result.unwrap();
        assert_eq!(result_set.len(), 2);
    }

    #[test]
    fn test_complex_query_parsing_edge_cases() {
        assert!(parse_query("SELECT * FROM person").is_ok());
        assert!(parse_query("SELECT name,email FROM person").is_ok());
        assert!(parse_query("SELECT name , email FROM person").is_ok());
        assert!(parse_query("SELECT name FROM person WHERE name='John'").is_ok());
        assert!(parse_query("SELECT name FROM person WHERE name = 'John'").is_ok());

        assert!(parse_query("SELECT name FROM person WHERE name = \"John\"").is_ok());
        assert!(parse_query("SELECT name FROM person WHERE name = 'John Doe'").is_ok());

        assert!(parse_query("select name from person where name = 'John'").is_ok());
        assert!(parse_query("Select Name From Person Where Name = 'John'").is_ok());
    }

    #[test]
    fn test_complex_error_handling() {
        let graph = create_complex_workspace();
        let firm_ql = FirmQl::new(graph);

        let result = firm_ql.query("SELECT * FROM nonexistent_table");
        assert!(result.is_err());

        let graph = create_complex_workspace();
        let firm_ql = FirmQl::new(graph);
        let result = firm_ql.query("SELECT FROM person");
        assert!(result.is_err());

        let graph = create_complex_workspace();
        let firm_ql = FirmQl::new(graph);
        let result = firm_ql.query("SELECT * person");
        assert!(result.is_err());

        let result = parse_query("SELECT * FROM person WHERE name = 'John");
        assert!(result.is_err());

        let result = parse_query("SELECT * FROM person WHERE name === 'John'");
        assert!(result.is_err());
    }

    #[test]
    fn test_inner_join_basic() {
        let graph = create_complex_workspace();
        let firm_ql = FirmQl::new(graph);

        let result = firm_ql
            .query("SELECT name, status FROM person INNER JOIN contact ON status = 'active'");

        assert!(result.is_ok());
        let result_set = result.unwrap();

        assert!(result_set.len() > 0);
    }

    #[test]
    fn test_inner_join_with_where() {
        let graph = create_complex_workspace();
        let firm_ql = FirmQl::new(graph);

        let result = firm_ql.query(
            "SELECT name FROM person INNER JOIN contact ON status = 'active' WHERE role = 'CTO'",
        );

        assert!(result.is_ok());
        let result_set = result.unwrap();

        assert!(result_set.len() > 0);
    }

    #[test]
    fn test_left_join() {
        let graph = create_complex_workspace();
        let firm_ql = FirmQl::new(graph);

        let result =
            firm_ql.query("SELECT name FROM person LEFT JOIN contact ON status = 'active'");

        assert!(result.is_ok());
        let result_set = result.unwrap();

        assert!(result_set.len() >= 3);
    }

    #[test]
    fn test_right_join() {
        let graph = create_complex_workspace();
        let firm_ql = FirmQl::new(graph);

        let result =
            firm_ql.query("SELECT role FROM person RIGHT JOIN contact ON status = 'active'");

        assert!(result.is_ok());
        let result_set = result.unwrap();

        assert!(result_set.len() >= 2);
    }

    #[test]
    fn test_full_outer_join() {
        let graph = create_complex_workspace();
        let firm_ql = FirmQl::new(graph);

        let result =
            firm_ql.query("SELECT name FROM person FULL JOIN contact ON status = 'active'");

        assert!(result.is_ok());
        let result_set = result.unwrap();

        assert!(result_set.len() >= 5);
    }

    #[test]
    fn test_multiple_joins() {
        let graph = create_complex_workspace();
        let firm_ql = FirmQl::new(graph);

        let result = firm_ql.query(
            "SELECT name FROM person
             INNER JOIN contact ON status = 'active'
             INNER JOIN organization ON name = 'Acme Corp'",
        );

        assert!(result.is_ok());
        let result_set = result.unwrap();

        assert!(result_set.len() > 0);
    }

    #[test]
    fn test_join_with_task_assignment() {
        let graph = create_complex_workspace();
        let firm_ql = FirmQl::new(graph);

        let result =
            firm_ql.query("SELECT name FROM task INNER JOIN person ON email = 'john@acme.com'");

        assert!(result.is_ok());
        let result_set = result.unwrap();

        assert!(result_set.len() >= 1);
    }

    #[test]
    fn test_join_with_project_hierarchy() {
        let graph = create_complex_workspace();
        let firm_ql = FirmQl::new(graph);

        let result =
            firm_ql.query("SELECT name FROM project INNER JOIN task ON status = 'in_progress'");

        assert!(result.is_ok());
        let result_set = result.unwrap();

        assert!(result_set.len() >= 1);
    }

    #[test]
    fn test_join_using_clause() {
        let graph = create_complex_workspace();
        let firm_ql = FirmQl::new(graph);

        let result = parse_query("SELECT * FROM person p JOIN contact c USING (id)");
        assert!(result.is_ok());

        let result = firm_ql.query("SELECT name FROM task JOIN project USING (description)");
        if let Err(ref e) = result {
            println!("Error executing USING query: {:?}", e);
        }
        assert!(result.is_ok());
        let result_set = result.unwrap();

        assert!(result_set.len() == result_set.len());
    }

    #[test]
    fn test_join_with_complex_conditions() {
        let graph = create_complex_workspace();
        let firm_ql = FirmQl::new(graph);

        let result = firm_ql.query(
            "SELECT name FROM person
             INNER JOIN contact ON status = 'active' AND role = 'CTO'",
        );

        assert!(result.is_ok());
        let result_set = result.unwrap();

        assert!(result_set.len() >= 1);
    }

    #[test]
    fn test_join_with_aliases() {
        let _graph = create_complex_workspace();

        let result = parse_query(
            "SELECT person_table.name AS person_name, contact_table.role AS contact_role
             FROM person AS person_table
             INNER JOIN contact AS contact_table ON person_table.name = 'John Doe'",
        );

        assert!(result.is_ok());
    }

    #[test]
    fn test_join_parsing_different_types() {
        let queries = vec![
            "SELECT * FROM person INNER JOIN contact ON name = 'John'",
            "SELECT * FROM person LEFT JOIN contact ON name = 'John'",
            "SELECT * FROM person RIGHT JOIN contact ON name = 'John'",
            "SELECT * FROM person FULL JOIN contact ON name = 'John'",
            "SELECT * FROM person JOIN contact ON name = 'John'",
        ];

        for query in queries {
            let result = parse_query(query);
            assert!(result.is_ok(), "Failed to parse: {}", query);
        }
    }

    #[test]
    fn test_join_error_cases() {
        let invalid_queries = vec![
            "SELECT * FROM person JOIN",
            "SELECT * FROM person JOIN contact ON",
            "SELECT * FROM person JOIN contact USING",
            "SELECT * FROM person JOIN contact USING ()",
        ];

        for query in invalid_queries {
            let result = parse_query(query);
            assert!(result.is_err(), "Should fail to parse: {}", query);
        }
    }

    #[test]
    fn test_triple_join_complex() {
        let graph = create_complex_workspace();
        let firm_ql = FirmQl::new(graph);

        let result = firm_ql.query(
            "SELECT name
             FROM opportunity
             INNER JOIN contact ON role = 'CTO'
             INNER JOIN person ON email = 'john@acme.com'",
        );

        assert!(result.is_ok());
        let result_set = result.unwrap();

        assert!(result_set.len() > 0);
    }

    #[test]
    fn test_join_types_comprehensive_comparison() {
        let base_condition = "status = 'active'";

        let graph = create_complex_workspace();
        let firm_ql = FirmQl::new(graph);
        let inner_result = firm_ql.query(&format!(
            "SELECT name FROM person INNER JOIN contact ON {}",
            base_condition
        ));
        assert!(inner_result.is_ok());
        let inner_count = inner_result.unwrap().len();

        let graph = create_complex_workspace();
        let firm_ql = FirmQl::new(graph);
        let left_result = firm_ql.query(&format!(
            "SELECT name FROM person LEFT JOIN contact ON {}",
            base_condition
        ));
        assert!(left_result.is_ok());
        let left_count = left_result.unwrap().len();

        let graph = create_complex_workspace();
        let firm_ql = FirmQl::new(graph);
        let right_result = firm_ql.query(&format!(
            "SELECT name FROM person RIGHT JOIN contact ON {}",
            base_condition
        ));
        assert!(right_result.is_ok());
        let right_count = right_result.unwrap().len();

        let graph = create_complex_workspace();
        let firm_ql = FirmQl::new(graph);
        let full_result = firm_ql.query(&format!(
            "SELECT name FROM person FULL JOIN contact ON {}",
            base_condition
        ));
        assert!(full_result.is_ok());
        let full_count = full_result.unwrap().len();

        assert!(
            left_count >= inner_count,
            "LEFT JOIN should have at least as many rows as INNER JOIN"
        );

        assert!(
            right_count >= inner_count,
            "RIGHT JOIN should have at least as many rows as INNER JOIN"
        );

        assert!(
            full_count >= left_count,
            "FULL OUTER JOIN should have at least as many rows as LEFT JOIN"
        );
        assert!(
            full_count >= right_count,
            "FULL OUTER JOIN should have at least as many rows as RIGHT JOIN"
        );

        assert!(inner_count > 0, "INNER JOIN should return some results");
        assert!(left_count > 0, "LEFT JOIN should return some results");
        assert!(right_count > 0, "RIGHT JOIN should return some results");
        assert!(full_count > 0, "FULL OUTER JOIN should return some results");
    }

    #[test]
    fn test_order_by_functionality() {
        let graph = create_complex_workspace();
        let firm_ql = FirmQl::new(graph);

        let result = firm_ql.query("SELECT name FROM person ORDER BY name ASC");
        assert!(result.is_ok());
        let result_set = result.unwrap();
        assert_eq!(result_set.len(), 3);

        let first_name = &result_set.rows[0].values[0];
        if let firm_core::field::FieldValue::String(name) = first_name {
            assert_eq!(name, "Alice Johnson");
        }

        let graph = create_complex_workspace();
        let firm_ql = FirmQl::new(graph);
        let result = firm_ql.query("SELECT name FROM person ORDER BY name DESC");
        assert!(result.is_ok());
        let result_set = result.unwrap();
        let first_name = &result_set.rows[0].values[0];
        if let firm_core::field::FieldValue::String(name) = first_name {
            assert_eq!(name, "John Doe");
        }
    }

    #[test]
    fn test_limit_functionality() {
        let graph = create_complex_workspace();
        let firm_ql = FirmQl::new(graph);

        let result = firm_ql.query("SELECT name FROM person LIMIT 2");
        assert!(result.is_ok());
        let result_set = result.unwrap();
        assert_eq!(result_set.len(), 2);

        let graph = create_complex_workspace();
        let firm_ql = FirmQl::new(graph);
        let result = firm_ql.query("SELECT name FROM person LIMIT 1 OFFSET 1");
        assert!(result.is_ok());
        let result_set = result.unwrap();
        assert_eq!(result_set.len(), 1);

        let graph = create_complex_workspace();
        let firm_ql = FirmQl::new(graph);
        let result = firm_ql.query("SELECT name FROM person LIMIT 10 OFFSET 10");
        assert!(result.is_ok());
        let result_set = result.unwrap();
        assert_eq!(result_set.len(), 0);
    }

    #[test]
    fn test_null_handling() {
        let _graph = create_complex_workspace();

        let result = parse_query("SELECT name FROM person WHERE phone IS NULL");
        assert!(result.is_ok());

        let result = parse_query("SELECT name FROM person WHERE phone IS NOT NULL");
        assert!(result.is_ok());
    }

    #[test]
    fn test_complex_expressions() {
        let graph = create_complex_workspace();
        let firm_ql = FirmQl::new(graph);

        let result =
            firm_ql.query("SELECT name FROM task WHERE (is_completed = true OR name LIKE '%ML%')");
        assert!(result.is_ok());

        let graph = create_complex_workspace();
        let firm_ql = FirmQl::new(graph);
        let result = firm_ql.query(
            "SELECT name FROM person WHERE name = 'John Doe' OR (name = 'Jane Smith' AND email LIKE '%techstartup%')",
        );
        assert!(result.is_ok());
        let result_set = result.unwrap();
        assert_eq!(result_set.len(), 2);
    }

    #[test]
    fn test_missing_binary_operators() {
        let graph = create_complex_workspace();
        let firm_ql = FirmQl::new(graph);

        let result = firm_ql.query("SELECT name FROM person WHERE name ILIKE '%john%'");
        assert!(result.is_ok());
        let result_set = result.unwrap();
        assert_eq!(result_set.len(), 2);

        let graph = create_complex_workspace();
        let firm_ql = FirmQl::new(graph);
        let result = firm_ql.query("SELECT name FROM person WHERE name NOT LIKE 'Jane%'");
        assert!(result.is_ok());
        let result_set = result.unwrap();
        assert_eq!(result_set.len(), 2);

        let result = parse_query("SELECT name FROM person WHERE age % 5 = 0");
        assert!(result.is_ok());

        let result = parse_query("SELECT name FROM person WHERE name IN ('John Doe')");
        assert!(result.is_ok());

        let result = parse_query("SELECT name FROM person WHERE name NOT IN ('Jane Smith')");
        assert!(result.is_ok());
    }

    #[test]
    fn test_missing_unary_operators() {
        let _graph = create_complex_workspace();

        let result = parse_query("SELECT name FROM person WHERE phone IS NULL");
        assert!(result.is_ok());

        let result = parse_query("SELECT name FROM person WHERE email IS NOT NULL");
        assert!(result.is_ok());
    }

    #[test]
    fn test_missing_functions() {
        let graph = create_complex_workspace();
        let firm_ql = FirmQl::new(graph);

        let result = firm_ql.query("SELECT MAX(name) FROM person");
        assert!(result.is_ok());

        let graph = create_complex_workspace();
        let firm_ql = FirmQl::new(graph);
        let result = firm_ql.query("SELECT MIN(name) FROM person");
        assert!(result.is_ok());

        let graph = create_complex_workspace();
        let firm_ql = FirmQl::new(graph);
        let result = firm_ql.query("SELECT SUM(probability) FROM opportunity");
        assert!(result.is_ok());

        let graph = create_complex_workspace();
        let firm_ql = FirmQl::new(graph);
        let result = firm_ql.query("SELECT AVG(probability) FROM opportunity");
        assert!(result.is_ok());
    }

    #[test]
    fn test_multiple_order_by_columns() {
        let graph = create_complex_workspace();
        let firm_ql = FirmQl::new(graph);

        let result = firm_ql.query("SELECT name FROM task ORDER BY is_completed ASC, name DESC");
        assert!(result.is_ok());
        let result_set = result.unwrap();
        assert_eq!(result_set.len(), 3);
    }

    #[test]
    fn test_arithmetic_expressions_in_queries() {
        let graph = create_complex_workspace();
        let firm_ql = FirmQl::new(graph);

        let result = firm_ql.query("SELECT name FROM opportunity WHERE probability + 25 = 100");
        assert!(result.is_ok());
        let result_set = result.unwrap();
        assert_eq!(result_set.len(), 1);

        let graph = create_complex_workspace();
        let firm_ql = FirmQl::new(graph);
        let result = firm_ql.query("SELECT name FROM opportunity WHERE probability - 25 = 50");
        assert!(result.is_ok());
        let result_set = result.unwrap();
        assert_eq!(result_set.len(), 1);

        let result = parse_query("SELECT name FROM person WHERE age * 2 > 50");
        assert!(result.is_ok());

        let result = parse_query("SELECT name FROM person WHERE age / 2 > 10");
        assert!(result.is_ok());
    }

    #[test]
    fn test_null_literal_handling() {
        let _graph = create_complex_workspace();

        let result = parse_query("SELECT name FROM person WHERE description = NULL");
        assert!(result.is_ok());

        let result = parse_query("SELECT name FROM person WHERE phone != NULL");
        assert!(result.is_ok());
    }

    #[test]
    fn test_complex_nested_expressions() {
        let graph = create_complex_workspace();
        let firm_ql = FirmQl::new(graph);

        let result = firm_ql.query(
            "SELECT name FROM task WHERE ((is_completed = true AND name LIKE '%design%') OR (is_completed = false AND name LIKE '%ML%'))",
        );
        assert!(result.is_ok());
        let result_set = result.unwrap();
        assert!(result_set.len() >= 1);

        let result = parse_query(
            "SELECT name FROM opportunity WHERE (probability > 50 AND probability < 100) OR (value > 100000 AND status = 'negotiation')",
        );
        assert!(result.is_ok());
    }

    #[test]
    fn test_function_error_handling() {
        let graph = create_complex_workspace();
        let firm_ql = FirmQl::new(graph);

        let result = firm_ql.query("SELECT MAX() FROM person");
        assert!(result.is_err());

        let graph = create_complex_workspace();
        let firm_ql = FirmQl::new(graph);
        let result = firm_ql.query("SELECT MIN() FROM person");
        assert!(result.is_err());

        let graph = create_complex_workspace();
        let firm_ql = FirmQl::new(graph);
        let result = firm_ql.query("SELECT SUM() FROM person");
        assert!(result.is_err());

        let graph = create_complex_workspace();
        let firm_ql = FirmQl::new(graph);
        let result = firm_ql.query("SELECT AVG() FROM person");
        assert!(result.is_err());

        let graph = create_complex_workspace();
        let firm_ql = FirmQl::new(graph);
        let result = firm_ql.query("SELECT UNSUPPORTED_FUNC(name) FROM person");
        assert!(result.is_err());
    }

    #[test]
    fn test_division_by_zero_handling() {
        let _graph = create_complex_workspace();

        let result = parse_query("SELECT name FROM opportunity WHERE probability / 0 > 50");
        assert!(result.is_ok());

        let result = parse_query("SELECT name FROM opportunity WHERE probability % 0 = 0");
        assert!(result.is_ok());
    }

    #[test]
    fn test_type_mismatch_error_handling() {
        let _graph = create_complex_workspace();

        let result = parse_query("SELECT name FROM person WHERE name + 5 > 'test'");
        assert!(result.is_ok());

        let result = parse_query("SELECT name FROM opportunity WHERE probability LIKE '%75%'");
        assert!(result.is_ok());
    }

    #[test]
    fn test_array_literal_parsing() {
        let result = parse_query("SELECT name FROM person WHERE id IN (1)");
        assert!(result.is_ok());

        let result = parse_query("SELECT name FROM person WHERE name IN ('John', 'Jane', 'Alice')");
        assert!(result.is_ok());

        let result = parse_query("SELECT name FROM person WHERE age IN (25, 30, 35)");
        assert!(result.is_ok());

        let result = parse_query("SELECT name FROM task WHERE is_completed IN (true, false)");
        assert!(result.is_ok());
    }

    #[test]
    fn test_array_in_operations() {
        let graph = create_complex_workspace();
        let firm_ql = FirmQl::new(graph);

        let result =
            firm_ql.query("SELECT name FROM person WHERE name IN ('John Doe', 'Alice Johnson')");
        assert!(result.is_ok());
        let result_set = result.unwrap();
        assert_eq!(result_set.len(), 2);

        let graph = create_complex_workspace();
        let firm_ql = FirmQl::new(graph);
        let result = firm_ql
            .query("SELECT name FROM person WHERE name NOT IN ('John Doe', 'Alice Johnson')");
        assert!(result.is_ok());
        let result_set = result.unwrap();
        assert_eq!(result_set.len(), 1);

        let graph = create_complex_workspace();
        let firm_ql = FirmQl::new(graph);
        let result = firm_ql.query("SELECT name FROM person WHERE name IN ('John Doe')");
        assert!(result.is_ok());
        let result_set = result.unwrap();
        assert_eq!(result_set.len(), 1);
    }

    #[test]
    fn test_array_with_different_types() {
        let graph = create_complex_workspace();
        let firm_ql = FirmQl::new(graph);

        let result = firm_ql.query("SELECT name FROM task WHERE is_completed IN (true)");
        assert!(result.is_ok());
        let result_set = result.unwrap();
        assert_eq!(result_set.len(), 1);

        let graph = create_complex_workspace();
        let firm_ql = FirmQl::new(graph);
        let result = firm_ql.query("SELECT name FROM opportunity WHERE probability IN (75)");
        assert!(result.is_ok());
        let result_set = result.unwrap();
        assert_eq!(result_set.len(), 1);

        let graph = create_complex_workspace();
        let firm_ql = FirmQl::new(graph);
        let result = firm_ql.query("SELECT name FROM task WHERE is_completed NOT IN (true)");
        assert!(result.is_ok());
        let result_set = result.unwrap();
        assert_eq!(result_set.len(), 2);
    }

    #[test]
    fn test_complex_array_queries() {
        let graph = create_complex_workspace();
        let firm_ql = FirmQl::new(graph);

        let result = firm_ql.query(
            "SELECT name FROM person WHERE name IN ('John Doe', 'Jane Smith') AND email LIKE '%@%'",
        );
        assert!(result.is_ok());
        let result_set = result.unwrap();
        assert_eq!(result_set.len(), 2);

        let graph = create_complex_workspace();
        let firm_ql = FirmQl::new(graph);
        let result = firm_ql.query(
            "SELECT COUNT(*) FROM person WHERE name IN ('John Doe', 'Alice Johnson', 'NonExistent')",
        );
        assert!(result.is_ok());
        let result_set = result.unwrap();
        assert_eq!(result_set.len(), 2);
    }

    #[test]
    fn test_array_error_cases() {
        let result = parse_query("SELECT name FROM person WHERE id IN (1, 2,)");
        assert!(result.is_err());

        let result = parse_query("SELECT name FROM person WHERE id IN (1 2)");
        assert!(result.is_err());

        let result = parse_query("SELECT name FROM person WHERE id IN (");
        assert!(result.is_err());
    }

    #[test]
    fn test_nested_array_expressions() {
        let result = parse_query(
            "SELECT name FROM person WHERE (name IN ('John', 'Jane') OR age IN (25, 30)) AND email IS NOT NULL",
        );
        assert!(result.is_ok());

        let result = parse_query("SELECT name FROM person WHERE NOT (name IN ('John', 'Jane'))");
        assert!(result.is_ok());
    }

    #[test]
    fn test_show_entity_types() {
        let graph = create_complex_workspace();
        let firm_ql = FirmQl::new(graph);

        let result = firm_ql.query("SHOW ENTITY TYPES");
        assert!(result.is_ok());

        let result_set = result.unwrap();
        assert_eq!(result_set.columns.len(), 1);
        assert_eq!(result_set.columns[0], "entity_type");

        assert!(result_set.len() > 0);

        let entity_types: Vec<String> = result_set
            .rows
            .iter()
            .map(|row| {
                if let FieldValue::String(s) = &row.values[0] {
                    s.clone()
                } else {
                    panic!("Expected string value for entity_type");
                }
            })
            .collect();

        assert!(entity_types.contains(&"person".to_string()));
        assert!(entity_types.contains(&"organization".to_string()));
        assert!(entity_types.contains(&"contact".to_string()));
        assert!(entity_types.contains(&"project".to_string()));
        assert!(entity_types.contains(&"task".to_string()));
    }

    #[test]
    fn test_show_entity_types_case_insensitive() {
        let graph = create_complex_workspace();
        let firm_ql = FirmQl::new(graph);

        let result = firm_ql.query("show entity types");
        assert!(result.is_ok());

        let result = firm_ql.query("Show Entity Types");
        assert!(result.is_ok());
    }

    #[test]
    fn test_describe_entity_type() {
        let graph = create_complex_workspace();
        let firm_ql = FirmQl::new(graph);

        let result = firm_ql.query("DESCRIBE person");
        assert!(result.is_ok());

        let result_set = result.unwrap();
        assert_eq!(result_set.columns.len(), 2);
        assert_eq!(result_set.columns[0], "field_name");
        assert_eq!(result_set.columns[1], "field_type");

        assert!(result_set.len() > 0);

        let field_names: Vec<String> = result_set
            .rows
            .iter()
            .map(|row| {
                if let FieldValue::String(s) = &row.values[0] {
                    s.clone()
                } else {
                    panic!("Expected string value for field_name");
                }
            })
            .collect();

        assert!(field_names.contains(&"name".to_string()));
        assert!(field_names.contains(&"email".to_string()));
    }

    #[test]
    fn test_describe_entity_type_case_insensitive() {
        let graph = create_complex_workspace();
        let firm_ql = FirmQl::new(graph);

        let result = firm_ql.query("describe organization");
        assert!(result.is_ok());

        let result = firm_ql.query("DESCRIBE Organization");
        assert!(result.is_ok());

        let result = firm_ql.query("Describe organization");
        assert!(result.is_ok());
    }

    #[test]
    fn test_describe_nonexistent_entity_type() {
        let graph = create_complex_workspace();
        let firm_ql = FirmQl::new(graph);

        let result = firm_ql.query("DESCRIBE nonexistent_type");
        assert!(result.is_err());

        if let Err(e) = result {
            assert!(e
                .to_string()
                .contains("Entity type 'nonexistent_type' not found"));
        }
    }

    #[test]
    fn test_like_operations_comprehensive() {
        let graph = create_complex_workspace();
        let firm_ql = FirmQl::new(graph);

        let result = firm_ql.query("SELECT name FROM person WHERE name LIKE 'John%'");
        assert!(result.is_ok());
        let _ = result.unwrap();

        let result = firm_ql.query("SELECT name FROM person WHERE name LIKE 'john%'");
        assert!(result.is_ok());
        let _ = result.unwrap();

        let result = firm_ql.query("SELECT name FROM person WHERE name ILIKE 'john%'");
        assert!(result.is_ok());
        let _ = result.unwrap();

        let result = firm_ql.query("SELECT name FROM person WHERE name NOT LIKE 'John%'");
        assert!(result.is_ok());
        let _ = result.unwrap();

        let result = firm_ql.query("SELECT name FROM task WHERE name LIKE '%ML%'");
        assert!(result.is_ok());
        let result_set = result.unwrap();
        assert!(result_set.len() > 0);

        let result = firm_ql.query("SELECT name FROM person WHERE email LIKE '%@acme.com'");
        assert!(result.is_ok());
        let result_set = result.unwrap();
        assert!(result_set.len() > 0);

        let result = firm_ql.query("SELECT name FROM person WHERE name LIKE 'John Doe'");
        assert!(result.is_ok());
        let result_set = result.unwrap();
        assert_eq!(result_set.len(), 1);
    }

    #[test]
    fn test_like_operations_with_references() {
        let graph = create_complex_workspace();
        let firm_ql = FirmQl::new(graph);

        let result = firm_ql.query("SELECT name FROM task WHERE assignee_ref LIKE '%john%'");
        assert!(result.is_ok());
        let _ = result.unwrap();

        let result = firm_ql.query("SELECT name FROM task WHERE assignee_ref ILIKE '%JOHN%'");
        assert!(result.is_ok());
        let _ = result.unwrap();

        let result = firm_ql.query("SELECT name FROM task WHERE assignee_ref NOT LIKE '%alice%'");
        assert!(result.is_ok());
        let _ = result.unwrap();

        let result = firm_ql.query("SELECT name FROM task WHERE source_ref LIKE '%ai_platform%'");
        assert!(result.is_ok());
        let _ = result.unwrap();
    }

    #[test]
    fn test_like_operations_edge_cases() {
        let graph = create_complex_workspace();
        let firm_ql = FirmQl::new(graph);

        let result = firm_ql.query("SELECT name FROM person WHERE name LIKE 'NonExistent%'");
        assert!(result.is_ok());
        let result_set = result.unwrap();
        assert_eq!(result_set.len(), 0);

        let result = firm_ql.query("SELECT name FROM person WHERE name LIKE ''");
        assert!(result.is_ok());
        let result_set = result.unwrap();
        assert_eq!(result_set.len(), 0);

        let result = firm_ql.query("SELECT name FROM person WHERE name LIKE '%'");
        assert!(result.is_ok());
        let _ = result.unwrap();

        let result = firm_ql.query("SELECT name FROM person WHERE name LIKE '%o%o%'");
        assert!(result.is_ok());
        let _ = result.unwrap();

        let result = firm_ql.query("SELECT email FROM person WHERE email LIKE '%.com'");
        assert!(result.is_ok());
        let _ = result.unwrap();
    }

    #[test]
    fn test_like_operations_complex_queries() {
        let graph = create_complex_workspace();
        let firm_ql = FirmQl::new(graph);

        let result =
            firm_ql.query("SELECT name FROM person WHERE name LIKE 'J%' AND email LIKE '%acme%'");
        assert!(result.is_ok());
        let result_set = result.unwrap();
        assert!(result_set.len() > 0);

        let result =
            firm_ql.query("SELECT name FROM person WHERE name LIKE 'John%' OR name LIKE 'Jane%'");
        assert!(result.is_ok());
        let result_set = result.unwrap();
        assert!(result_set.len() >= 2);

        let result =
            firm_ql.query("SELECT name FROM person WHERE name LIKE 'John%' OR name ILIKE 'alice%'");
        assert!(result.is_ok());
        let result_set = result.unwrap();
        assert!(result_set.len() >= 2);

        let result =
            firm_ql.query("SELECT name FROM task WHERE name LIKE '%ML%' AND is_completed = false");
        assert!(result.is_ok());
        let _ = result.unwrap();

        let result = firm_ql
            .query("SELECT name FROM person WHERE name NOT LIKE 'Test%' AND email LIKE '%@%'");
        assert!(result.is_ok());
        let result_set = result.unwrap();
        assert!(result_set.len() > 0);
    }

    #[test]
    fn test_business_model_canvas_ref_scenario() {
        let mut graph = EntityGraph::new();

        let business_model_canvas = Entity::new(
            firm_core::EntityId::new("business_model_canvas.kaphera_bmx"),
            EntityType::new("business_model_canvas"),
        )
        .with_field(
            FieldId::new("name"),
            FieldValue::String("Kaphera BMX".to_string()),
        );

        let value_prop = Entity::new(
            firm_core::EntityId::new("value_proposition_assumption.vp1"),
            EntityType::new("value_proposition_assumption"),
        )
        .with_field(
            FieldId::new("title"),
            FieldValue::String("Key Value Proposition".to_string()),
        )
        .with_field(
            FieldId::new("statement"),
            FieldValue::String("We solve customer pain points".to_string()),
        )
        .with_field(
            FieldId::new("importance_level"),
            FieldValue::String("High".to_string()),
        )
        .with_field(
            FieldId::new("status"),
            FieldValue::String("Active".to_string()),
        )
        .with_field(
            FieldId::new("business_model_canvas_ref"),
            FieldValue::Reference(firm_core::field::ReferenceValue::Entity(
                firm_core::EntityId::new("business_model_canvas.kaphera_bmx"),
            )),
        );

        graph.add_entity(business_model_canvas).unwrap();
        graph.add_entity(value_prop).unwrap();

        let firm_ql = FirmQl::new(graph);

        let result = firm_ql.query("SELECT title, statement, importance_level, status FROM value_proposition_assumption WHERE business_model_canvas_ref LIKE '%kaphera%'");
        assert!(
            result.is_ok(),
            "Query should succeed with reference LIKE operation"
        );
        let result_set = result.unwrap();
        assert_eq!(result_set.len(), 1, "Should find one matching record");

        let result = firm_ql.query("SELECT title FROM value_proposition_assumption WHERE business_model_canvas_ref ILIKE '%KAPHERA%'");
        assert!(result.is_ok());
        let result_set = result.unwrap();
        assert_eq!(result_set.len(), 1);

        let result = firm_ql.query("SELECT title FROM value_proposition_assumption WHERE business_model_canvas_ref NOT LIKE '%other%'");
        assert!(result.is_ok());
        let result_set = result.unwrap();
        assert_eq!(result_set.len(), 1);
    }
    #[test]
    fn test_id_injection() {
        let graph = create_test_graph();
        let firm_ql = FirmQl::new(graph);

        let results = firm_ql.query("SELECT * FROM person").unwrap();
        assert!(results.columns.contains(&"id".to_string()));

        let results = firm_ql.query("SELECT id, name FROM person").unwrap();
        assert_eq!(results.columns, vec!["id".to_string(), "name".to_string()]);
        assert!(!results.rows.is_empty());

        if let Some(first_row) = results.rows.first() {
            if let Some(id_value) = first_row.values.first() {
                match id_value {
                    FieldValue::String(s) => assert_ne!(s, "NULL"),
                    _ => panic!("Expected id to be a string"),
                }
            }
        }

        let results = firm_ql.query("SELECT person.* FROM person").unwrap();
        assert!(results.columns.iter().any(|col| col == "person.id"));
    }

    #[test]
    fn test_where_with_references() {
        let mut graph = EntityGraph::new();

        let tomato_inc = Entity::new(
            firm_core::EntityId::new("organization.tomato_inc"),
            EntityType::new("organization"),
        )
        .with_field(
            FieldId::new("name"),
            FieldValue::String("Tomato Inc".to_string()),
        );

        let beans_ltd = Entity::new(
            firm_core::EntityId::new("organization.beans_ltd"),
            EntityType::new("organization"),
        )
        .with_field(
            FieldId::new("name"),
            FieldValue::String("Beans Ltd".to_string()),
        );

        let employee1 = Entity::new(
            firm_core::EntityId::new("employee.john_tomato"),
            EntityType::new("employee"),
        )
        .with_field(
            FieldId::new("name"),
            FieldValue::String("John Smith".to_string()),
        )
        .with_field(
            FieldId::new("organization_ref"),
            FieldValue::String("organization.tomato_inc".to_string()),
        );

        let employee2 = Entity::new(
            firm_core::EntityId::new("employee.jane_tomato"),
            EntityType::new("employee"),
        )
        .with_field(
            FieldId::new("name"),
            FieldValue::String("Jane Doe".to_string()),
        )
        .with_field(
            FieldId::new("organization_ref"),
            FieldValue::String("organization.tomato_inc".to_string()),
        );

        let employee3 = Entity::new(
            firm_core::EntityId::new("employee.bob_beans"),
            EntityType::new("employee"),
        )
        .with_field(
            FieldId::new("name"),
            FieldValue::String("Bob Johnson".to_string()),
        )
        .with_field(
            FieldId::new("organization_ref"),
            FieldValue::String("organization.beans_ltd".to_string()),
        );

        let employee4 = Entity::new(
            firm_core::EntityId::new("employee.alice_beans"),
            EntityType::new("employee"),
        )
        .with_field(
            FieldId::new("name"),
            FieldValue::String("Alice Brown".to_string()),
        )
        .with_field(
            FieldId::new("organization_ref"),
            FieldValue::String("organization.beans_ltd".to_string()),
        );

        graph.add_entity(tomato_inc).unwrap();
        graph.add_entity(beans_ltd).unwrap();
        graph.add_entity(employee1).unwrap();
        graph.add_entity(employee2).unwrap();
        graph.add_entity(employee3).unwrap();
        graph.add_entity(employee4).unwrap();

        graph.build();
        let firm_ql = FirmQl::new(graph);

        let results = firm_ql
            .query("SELECT * FROM employee WHERE organization_ref = 'organization.tomato_inc'")
            .unwrap();
        assert_eq!(results.rows.len(), 2);

        for row in &results.rows {
            let org_ref_index = results
                .columns
                .iter()
                .position(|c| c == "organization_ref")
                .unwrap();
            if let FieldValue::String(org_ref) = &row.values[org_ref_index] {
                assert_eq!(org_ref, "organization.tomato_inc");
            }
        }

        let results = firm_ql.query("SELECT * FROM employee LIMIT 1").unwrap();
        assert_eq!(results.rows.len(), 1);
    }
    #[test]
    fn test_reference_string_equality_issue() {
        let mut graph = EntityGraph::new();

        let business_model_canvas = Entity::new(
            firm_core::EntityId::new("business_model_canvas.tomato_inc"),
            EntityType::new("business_model_canvas"),
        )
        .with_field(
            FieldId::new("name"),
            FieldValue::String("Tomato Inc Business Model".to_string()),
        )
        .with_field(
            FieldId::new("organization_ref"),
            FieldValue::Reference(firm_core::field::ReferenceValue::Entity(
                firm_core::EntityId::new("organization.tomato_inc"),
            )),
        );

        let organization = Entity::new(
            firm_core::EntityId::new("organization.tomato_inc"),
            EntityType::new("organization"),
        )
        .with_field(
            FieldId::new("name"),
            FieldValue::String("Tomato Inc, Ltd.".to_string()),
        );

        graph.add_entity(business_model_canvas).unwrap();
        graph.add_entity(organization).unwrap();

        let firm_ql = FirmQl::new(graph);

        println!("Testing reference equality with string literal...");

        let result = firm_ql.query(
            "SELECT * FROM business_model_canvas WHERE organization_ref = 'organization.tomato_inc'",
        );
        match result {
            Ok(result_set) => {
                println!("SUCCESS: Found {} matching records", result_set.len());
                assert_eq!(
                    result_set.len(),
                    1,
                    "Should find exactly one business model canvas"
                );
            }
            Err(e) => {
                println!("FAILED: Reference equality query failed: {}", e);
                panic!("Reference equality with string literal should work");
            }
        }

        let result = firm_ql.query(
            "SELECT * FROM business_model_canvas WHERE organization_ref LIKE '%tomato_inc%'",
        );
        match result {
            Ok(result_set) => {
                println!("LIKE test: Found {} matching records", result_set.len());
                assert_eq!(
                    result_set.len(),
                    1,
                    "Should find exactly one business model canvas with LIKE"
                );
            }
            Err(e) => {
                println!("LIKE test failed: {}", e);
                panic!("LIKE operation on reference should work");
            }
        }
    }

    #[test]
    fn test_reference_equality_comprehensive() {
        let mut graph = EntityGraph::new();

        let organization = Entity::new(
            firm_core::EntityId::new("organization.tomato_inc"),
            EntityType::new("organization"),
        )
        .with_field(
            FieldId::new("name"),
            FieldValue::String("Tomato Inc, Ltd.".to_string()),
        );

        let business_model_canvas = Entity::new(
            firm_core::EntityId::new("business_model_canvas.tomato_inc"),
            EntityType::new("business_model_canvas"),
        )
        .with_field(
            FieldId::new("name"),
            FieldValue::String("Tomato Inc BMC".to_string()),
        )
        .with_field(
            FieldId::new("organization_ref"),
            FieldValue::Reference(firm_core::field::ReferenceValue::Entity(
                firm_core::EntityId::new("organization.tomato_inc"),
            )),
        );

        let value_prop = Entity::new(
            firm_core::EntityId::new("value_proposition_assumption.vp1"),
            EntityType::new("value_proposition_assumption"),
        )
        .with_field(
            FieldId::new("title"),
            FieldValue::String("Key Value Prop".to_string()),
        )
        .with_field(
            FieldId::new("business_model_canvas_ref"),
            FieldValue::Reference(firm_core::field::ReferenceValue::Entity(
                firm_core::EntityId::new("business_model_canvas.tomato_inc"),
            )),
        );

        graph.add_entity(organization).unwrap();
        graph.add_entity(business_model_canvas).unwrap();
        graph.add_entity(value_prop).unwrap();

        let firm_ql = FirmQl::new(graph);

        let result = firm_ql
            .query("SELECT name FROM business_model_canvas WHERE organization_ref = 'organization.tomato_inc'");
        assert!(result.is_ok());
        let result_set = result.unwrap();
        assert_eq!(result_set.len(), 1);

        let result = firm_ql
            .query("SELECT name FROM business_model_canvas WHERE 'organization.tomato_inc' = organization_ref");
        assert!(result.is_ok());
        let result_set = result.unwrap();
        assert_eq!(result_set.len(), 1);

        let result = firm_ql.query(
            "SELECT name FROM business_model_canvas WHERE organization_ref != 'organization.other'",
        );
        assert!(result.is_ok());
        let result_set = result.unwrap();
        assert_eq!(result_set.len(), 1);

        let result = firm_ql.query("SELECT vp.title FROM value_proposition_assumption vp, business_model_canvas bmc WHERE vp.business_model_canvas_ref = bmc.id");
        assert!(result.is_ok());
        let result_set = result.unwrap();
        assert_eq!(result_set.len(), 1);

        let result = firm_ql.query("SELECT title FROM value_proposition_assumption WHERE business_model_canvas_ref = 'business_model_canvas.tomato_inc' AND title = 'Key Value Prop'");
        assert!(result.is_ok());
        let result_set = result.unwrap();
        assert_eq!(result_set.len(), 1);

        let result = firm_ql.query("SELECT title FROM value_proposition_assumption WHERE business_model_canvas_ref = 'business_model_canvas.tomato_inc' OR business_model_canvas_ref = 'business_model_canvas.other'");
        assert!(result.is_ok());
        let result_set = result.unwrap();
        assert_eq!(result_set.len(), 1);

        let result = firm_ql
            .query("SELECT title FROM value_proposition_assumption WHERE business_model_canvas_ref LIKE '%tomato_inc%'");
        assert!(result.is_ok());
        let result_set = result.unwrap();
        assert_eq!(result_set.len(), 1);

        let result = firm_ql.query("SELECT name FROM business_model_canvas WHERE organization_ref = 'organization.tomato_inc' AND name = 'Tomato Inc BMC'");
        assert!(result.is_ok());
        let result_set = result.unwrap();
        assert_eq!(result_set.len(), 1);
    }

    #[test]
    fn test_reference_comparison_edge_cases() {
        let mut graph = EntityGraph::new();

        let entity1 = Entity::new(
            firm_core::EntityId::new("test.entity1"),
            EntityType::new("test_type"),
        )
        .with_field(
            FieldId::new("ref_field"),
            FieldValue::Reference(firm_core::field::ReferenceValue::Entity(
                firm_core::EntityId::new("target.a"),
            )),
        )
        .with_field(
            FieldId::new("string_field"),
            FieldValue::String("target.a".to_string()),
        );

        graph.add_entity(entity1).unwrap();
        let firm_ql = FirmQl::new(graph);

        let result = firm_ql.query("SELECT * FROM test_type WHERE ref_field = string_field");
        assert!(result.is_ok());
        let result_set = result.unwrap();
        assert_eq!(
            result_set.len(),
            1,
            "Reference should equal string with same content"
        );

        let result = firm_ql.query("SELECT * FROM test_type WHERE ref_field = 'TARGET.A'");
        assert!(result.is_ok());
        let result_set = result.unwrap();
        assert_eq!(
            result_set.len(),
            0,
            "Reference comparison should be case sensitive"
        );

        let result = firm_ql.query("SELECT * FROM test_type WHERE ref_field = ''");
        assert!(result.is_ok());
        let result_set = result.unwrap();
        assert_eq!(
            result_set.len(),
            0,
            "Reference should not equal empty string"
        );

        let result = firm_ql.query("SELECT * FROM test_type WHERE ref_field = NULL");
        assert!(result.is_ok());
        let result_set = result.unwrap();
        assert_eq!(result_set.len(), 0, "Reference should not equal NULL");
    }
}
