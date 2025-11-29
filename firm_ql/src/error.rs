use thiserror::Error;

/// Result type for query operations.
pub type QueryResult<T> = Result<T, QueryError>;

/// Errors that can occur during query parsing and execution.
#[derive(Error, Debug)]
pub enum QueryError {
    /// Syntax error in the query string.
    #[error("Syntax error: {message} at position {position}")]
    SyntaxError { message: String, position: usize },

    /// Unknown entity type referenced in the query.
    #[error("Unknown entity type: {entity_type}")]
    UnknownEntityType { entity_type: String },

    /// Unknown field referenced in the query.
    #[error("Unknown field '{field}' on entity type '{entity_type}'")]
    UnknownField { field: String, entity_type: String },

    /// Type mismatch in comparison operations.
    #[error("Type mismatch: cannot compare {left_type} with {right_type}")]
    TypeMismatch {
        left_type: String,
        right_type: String,
    },

    /// Invalid function call or arguments.
    #[error("Invalid function: {function}({args})")]
    InvalidFunction { function: String, args: String },

    /// Runtime error during query execution.
    #[error("Execution error: {message}")]
    ExecutionError { message: String },

    /// Error when parsing literal values.
    #[error("Invalid literal: {literal}")]
    InvalidLiteral { literal: String },

    /// Error when a required entity reference cannot be resolved.
    #[error("Cannot resolve reference: {reference}")]
    UnresolvedReference { reference: String },

    /// Error when attempting an unsupported operation.
    #[error("Unsupported operation: {operation}")]
    UnsupportedOperation { operation: String },

    /// Error when query contains invalid SQL syntax.
    #[error("Invalid SQL syntax: {details}")]
    InvalidSyntax { details: String },

    /// Error when accessing non-existent columns in result set.
    #[error("Column '{column}' not found in result set")]
    ColumnNotFound { column: String },
}

impl QueryError {
    /// Create a new syntax error.
    pub fn syntax(message: impl Into<String>, position: usize) -> Self {
        Self::SyntaxError {
            message: message.into(),
            position,
        }
    }

    /// Create a new unknown entity type error.
    pub fn unknown_entity_type(entity_type: impl Into<String>) -> Self {
        Self::UnknownEntityType {
            entity_type: entity_type.into(),
        }
    }

    /// Create a new unknown field error.
    pub fn unknown_field(field: impl Into<String>, entity_type: impl Into<String>) -> Self {
        Self::UnknownField {
            field: field.into(),
            entity_type: entity_type.into(),
        }
    }

    /// Create a new type mismatch error.
    pub fn type_mismatch(left_type: impl Into<String>, right_type: impl Into<String>) -> Self {
        Self::TypeMismatch {
            left_type: left_type.into(),
            right_type: right_type.into(),
        }
    }

    /// Create a new execution error.
    pub fn execution(message: impl Into<String>) -> Self {
        Self::ExecutionError {
            message: message.into(),
        }
    }

    /// Create a new invalid literal error.
    pub fn invalid_literal(literal: impl Into<String>) -> Self {
        Self::InvalidLiteral {
            literal: literal.into(),
        }
    }

    /// Create a new unresolved reference error.
    pub fn unresolved_reference(reference: impl Into<String>) -> Self {
        Self::UnresolvedReference {
            reference: reference.into(),
        }
    }

    /// Create a new unsupported operation error.
    pub fn unsupported_operation(operation: impl Into<String>) -> Self {
        Self::UnsupportedOperation {
            operation: operation.into(),
        }
    }

    /// Create a new invalid syntax error.
    pub fn invalid_syntax(details: impl Into<String>) -> Self {
        Self::InvalidSyntax {
            details: details.into(),
        }
    }

    /// Create a new column not found error.
    pub fn column_not_found(column: impl Into<String>) -> Self {
        Self::ColumnNotFound {
            column: column.into(),
        }
    }
}
