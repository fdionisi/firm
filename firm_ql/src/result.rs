use crate::error::{QueryError, QueryResult};
use firm_core::field::FieldValue;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fmt;

/// Represents the result of a query execution.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QueryResultSet {
    /// Column names in the result set
    pub columns: Vec<String>,
    /// Rows of data
    pub rows: Vec<QueryRow>,
    /// Metadata about the query execution
    pub metadata: QueryMetadata,
}

impl QueryResultSet {
    /// Create a new empty result set.
    pub fn new(columns: Vec<String>) -> Self {
        Self {
            columns,
            rows: Vec::new(),
            metadata: QueryMetadata::default(),
        }
    }

    /// Create a new result set with rows.
    pub fn with_rows(columns: Vec<String>, rows: Vec<QueryRow>) -> Self {
        Self {
            columns,
            rows,
            metadata: QueryMetadata::default(),
        }
    }

    /// Add a row to the result set.
    pub fn add_row(&mut self, row: QueryRow) -> QueryResult<()> {
        if row.values.len() != self.columns.len() {
            return Err(QueryError::execution(format!(
                "Row has {} values but expected {} columns",
                row.values.len(),
                self.columns.len()
            )));
        }
        self.rows.push(row);
        Ok(())
    }

    /// Get the number of rows in the result set.
    pub fn len(&self) -> usize {
        self.rows.len()
    }

    /// Check if the result set is empty.
    pub fn is_empty(&self) -> bool {
        self.rows.is_empty()
    }

    /// Get a column index by name.
    pub fn column_index(&self, column_name: &str) -> QueryResult<usize> {
        self.columns
            .iter()
            .position(|c| c == column_name)
            .ok_or_else(|| QueryError::column_not_found(column_name))
    }

    /// Get all values for a specific column.
    pub fn column_values(&self, column_name: &str) -> QueryResult<Vec<&FieldValue>> {
        let index = self.column_index(column_name)?;
        Ok(self.rows.iter().map(|row| &row.values[index]).collect())
    }

    /// Convert to a vector of HashMaps (column_name -> value).
    pub fn to_maps(&self) -> Vec<HashMap<String, FieldValue>> {
        self.rows
            .iter()
            .map(|row| {
                self.columns
                    .iter()
                    .zip(row.values.iter())
                    .map(|(col, val)| (col.clone(), val.clone()))
                    .collect()
            })
            .collect()
    }

    /// Filter rows based on a predicate function.
    pub fn filter<F>(&self, predicate: F) -> Self
    where
        F: Fn(&QueryRow) -> bool,
    {
        let filtered_rows: Vec<QueryRow> = self
            .rows
            .iter()
            .filter(|row| predicate(row))
            .cloned()
            .collect();
        Self {
            columns: self.columns.clone(),
            rows: filtered_rows,
            metadata: self.metadata.clone(),
        }
    }

    /// Sort rows by a column.
    pub fn sort_by_column(&mut self, column_name: &str, ascending: bool) -> QueryResult<()> {
        let index = self.column_index(column_name)?;

        self.rows.sort_by(|a, b| {
            let comparison = compare_field_values(&a.values[index], &b.values[index]);
            if ascending {
                comparison
            } else {
                comparison.reverse()
            }
        });
        Ok(())
    }

    /// Sort rows by multiple columns with their respective sort directions.
    pub fn sort_by_columns(&mut self, sort_keys: &[(String, bool)]) -> QueryResult<()> {
        if sort_keys.is_empty() {
            return Ok(());
        }

        // Get column indices for all sort keys
        let mut column_indices = Vec::new();
        for (column_name, _) in sort_keys {
            let index = self.column_index(column_name)?;
            column_indices.push(index);
        }

        self.rows.sort_by(|a, b| {
            // Compare by each sort key in order
            for ((_, ascending), &col_index) in sort_keys.iter().zip(column_indices.iter()) {
                let comparison = compare_field_values(&a.values[col_index], &b.values[col_index]);
                let ordered_comparison = if *ascending {
                    comparison
                } else {
                    comparison.reverse()
                };

                // If this comparison is not equal, return it
                if ordered_comparison != std::cmp::Ordering::Equal {
                    return ordered_comparison;
                }
                // Otherwise, continue to next sort key
            }
            std::cmp::Ordering::Equal
        });
        Ok(())
    }

    /// Limit the number of rows returned.
    pub fn limit(&self, count: usize, offset: Option<usize>) -> Self {
        let start = offset.unwrap_or(0);
        let end = std::cmp::min(start + count, self.rows.len());

        if start >= self.rows.len() {
            return Self::new(self.columns.clone());
        }

        let limited_rows = self.rows[start..end].to_vec();
        Self {
            columns: self.columns.clone(),
            rows: limited_rows,
            metadata: self.metadata.clone(),
        }
    }
}

impl fmt::Display for QueryResultSet {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // Header
        writeln!(f, "{}", self.columns.join(" | "))?;
        writeln!(f, "{}", "-".repeat(self.columns.join(" | ").len()))?;

        // Rows
        for row in &self.rows {
            let row_values: Vec<String> = row.values.iter().map(|v| format!("{}", v)).collect();
            writeln!(f, "{}", row_values.join(" | "))?;
        }

        writeln!(f, "\n({} rows)", self.rows.len())?;
        Ok(())
    }
}

/// Represents a single row in a query result.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QueryRow {
    pub values: Vec<FieldValue>,
}

impl QueryRow {
    /// Create a new row with the given values.
    pub fn new(values: Vec<FieldValue>) -> Self {
        Self { values }
    }

    /// Get a value by column index.
    pub fn get(&self, index: usize) -> QueryResult<&FieldValue> {
        self.values
            .get(index)
            .ok_or_else(|| QueryError::execution(format!("Column index {} out of bounds", index)))
    }

    /// Get a value by column name (requires the result set for column mapping).
    pub fn get_by_name(
        &self,
        result_set: &QueryResultSet,
        column_name: &str,
    ) -> QueryResult<&FieldValue> {
        let index = result_set.column_index(column_name)?;
        self.get(index)
    }

    /// Convert the row to a HashMap with column names as keys.
    pub fn to_map(&self, columns: &[String]) -> HashMap<String, FieldValue> {
        columns
            .iter()
            .zip(self.values.iter())
            .map(|(col, val)| (col.clone(), val.clone()))
            .collect()
    }
}

/// Metadata about query execution.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QueryMetadata {
    /// Execution time in milliseconds
    pub execution_time_ms: Option<u64>,
    /// Number of entities scanned
    pub entities_scanned: Option<usize>,
    /// Query plan or execution details
    pub execution_plan: Option<String>,
    /// Whether the result was truncated
    pub truncated: bool,
}

impl Default for QueryMetadata {
    fn default() -> Self {
        Self {
            execution_time_ms: None,
            entities_scanned: None,
            execution_plan: None,
            truncated: false,
        }
    }
}

/// Compare two FieldValues for sorting purposes.
fn compare_field_values(a: &FieldValue, b: &FieldValue) -> std::cmp::Ordering {
    use std::cmp::Ordering;
    use FieldValue::*;

    match (a, b) {
        // Same types
        (String(a), String(b)) => a.cmp(b),
        (Integer(a), Integer(b)) => a.cmp(b),
        (Float(a), Float(b)) => a.partial_cmp(b).unwrap_or(Ordering::Equal),
        (Boolean(a), Boolean(b)) => a.cmp(b),
        (DateTime(a), DateTime(b)) => a.cmp(b),

        // Mixed numeric types
        (Integer(a), Float(b)) => (*a as f64).partial_cmp(b).unwrap_or(Ordering::Equal),
        (Float(a), Integer(b)) => a.partial_cmp(&(*b as f64)).unwrap_or(Ordering::Equal),

        // Different types - order by type precedence
        (String(_), _) => Ordering::Less,
        (_, String(_)) => Ordering::Greater,
        (Integer(_), _) => Ordering::Less,
        (_, Integer(_)) => Ordering::Greater,
        (Float(_), _) => Ordering::Less,
        (_, Float(_)) => Ordering::Greater,
        (Boolean(_), _) => Ordering::Less,
        (_, Boolean(_)) => Ordering::Greater,
        (DateTime(_), _) => Ordering::Less,
        (_, DateTime(_)) => Ordering::Greater,

        // For other types, consider them equal
        _ => Ordering::Equal,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use firm_core::field::FieldValue;

    #[test]
    fn test_result_set_creation() {
        let columns = vec!["name".to_string(), "age".to_string()];
        let result_set = QueryResultSet::new(columns.clone());

        assert_eq!(result_set.columns, columns);
        assert!(result_set.is_empty());
        assert_eq!(result_set.len(), 0);
    }

    #[test]
    fn test_add_row() {
        let mut result_set = QueryResultSet::new(vec!["name".to_string(), "age".to_string()]);

        let row = QueryRow::new(vec![
            FieldValue::String("John".to_string()),
            FieldValue::Integer(30),
        ]);

        result_set.add_row(row).unwrap();
        assert_eq!(result_set.len(), 1);
    }

    #[test]
    fn test_column_index() {
        let result_set = QueryResultSet::new(vec!["name".to_string(), "age".to_string()]);

        assert_eq!(result_set.column_index("name").unwrap(), 0);
        assert_eq!(result_set.column_index("age").unwrap(), 1);
        assert!(result_set.column_index("unknown").is_err());
    }

    #[test]
    fn test_compare_field_values() {
        use std::cmp::Ordering;

        let str1 = FieldValue::String("apple".to_string());
        let str2 = FieldValue::String("banana".to_string());
        assert_eq!(compare_field_values(&str1, &str2), Ordering::Less);

        let int1 = FieldValue::Integer(10);
        let int2 = FieldValue::Integer(20);
        assert_eq!(compare_field_values(&int1, &int2), Ordering::Less);

        let float1 = FieldValue::Float(1.5);
        let float2 = FieldValue::Float(2.5);
        assert_eq!(compare_field_values(&float1, &float2), Ordering::Less);
    }

    #[test]
    fn test_limit() {
        let mut result_set = QueryResultSet::new(vec!["id".to_string()]);

        for i in 0..10 {
            let row = QueryRow::new(vec![FieldValue::Integer(i)]);
            result_set.add_row(row).unwrap();
        }

        let limited = result_set.limit(5, None);
        assert_eq!(limited.len(), 5);

        let offset_limited = result_set.limit(3, Some(2));
        assert_eq!(offset_limited.len(), 3);
    }

    #[test]
    fn test_to_maps() {
        let mut result_set = QueryResultSet::new(vec!["name".to_string(), "age".to_string()]);

        let row = QueryRow::new(vec![
            FieldValue::String("John".to_string()),
            FieldValue::Integer(30),
        ]);
        result_set.add_row(row).unwrap();

        let maps = result_set.to_maps();
        assert_eq!(maps.len(), 1);
        assert_eq!(maps[0]["name"], FieldValue::String("John".to_string()));
        assert_eq!(maps[0]["age"], FieldValue::Integer(30));
    }
}
