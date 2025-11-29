use crate::error::{QueryError, QueryResult};
use crate::result::{QueryResultSet, QueryRow};
use firm_core::field::FieldValue;
use firm_core::{graph::EntityGraph, Entity, EntityType, FieldId};
use sqlparser::ast::{
    BinaryOperator, Expr, Function, FunctionArg, FunctionArgExpr, FunctionArguments,
    JoinConstraint, JoinOperator, ObjectName, OrderByExpr, Query, Select, SelectItem, SetExpr,
    TableFactor, TableWithJoins, UnaryOperator, Value,
};
use std::collections::HashMap;

/// Query execution engine that runs parsed queries against an EntityGraph.
pub struct QueryEngine {}

impl QueryEngine {
    /// Create a new query engine.
    pub fn new() -> Self {
        Self {}
    }

    /// Execute a parsed query and return results.
    pub fn execute(&self, query: &Query, graph: &EntityGraph) -> QueryResult<QueryResultSet> {
        match &*query.body {
            SetExpr::Select(select) => self.execute_select(select, query, graph),
            _ => Err(QueryError::syntax(
                "Only SELECT statements are supported",
                0,
            )),
        }
    }

    /// Execute a SELECT statement.
    fn execute_select(
        &self,
        select: &Select,
        query: &Query,
        graph: &EntityGraph,
    ) -> QueryResult<QueryResultSet> {
        let start_time = std::time::Instant::now();
        let mut entities_scanned = 0;

        let mut current_entities = self.get_base_entities(&select.from, graph)?;
        entities_scanned += current_entities.len();

        if let Some(selection) = &select.selection {
            current_entities = self.filter_entities(&current_entities, selection, graph)?;
        }

        let columns = self.build_columns_with_wildcards(&select.projection, &current_entities)?;

        if let Some(order_by) = &query.order_by {
            current_entities = self.sort_entities(current_entities, &order_by.exprs, graph)?;
        }

        if let Some(limit) = &query.limit {
            let limit_count = self.extract_limit_value(limit)?;
            let offset = if let Some(offset) = &query.offset {
                self.extract_limit_value(&offset.value)?
            } else {
                0
            };

            current_entities = current_entities
                .into_iter()
                .skip(offset as usize)
                .take(limit_count as usize)
                .collect();
        }

        let mut result_set = QueryResultSet::new(columns);
        result_set.metadata.execution_time_ms = Some(start_time.elapsed().as_millis() as u64);
        result_set.metadata.entities_scanned = Some(entities_scanned);

        for entity_map in current_entities {
            let values = self.extract_row_values(
                &select.projection,
                &entity_map,
                graph,
                &result_set.columns,
            )?;
            let row = QueryRow::new(values);
            result_set.add_row(row)?;
        }

        Ok(result_set)
    }

    /// Get base entities from FROM clause with JOIN support.
    fn get_base_entities(
        &self,
        from: &[TableWithJoins],
        graph: &EntityGraph,
    ) -> QueryResult<Vec<HashMap<String, Entity>>> {
        if from.is_empty() {
            return Err(QueryError::syntax("FROM clause is required", 0));
        }

        let first_table = &from[0];
        let mut current_entities = self.get_table_entities(&first_table.relation, graph)?;

        for join in &first_table.joins {
            current_entities = self.apply_join(
                &current_entities,
                &join.relation,
                &join.join_operator,
                graph,
            )?;
        }

        for table_with_joins in &from[1..] {
            let additional_entities = self.get_table_entities(&table_with_joins.relation, graph)?;
            current_entities = self.cross_join(&current_entities, &additional_entities);
        }

        Ok(current_entities)
    }

    /// Get entities for a single table.
    fn get_table_entities(
        &self,
        table_factor: &TableFactor,
        graph: &EntityGraph,
    ) -> QueryResult<Vec<HashMap<String, Entity>>> {
        match table_factor {
            TableFactor::Table { name, alias, .. } => {
                let table_name = self.object_name_to_string(name);
                let entity_type = EntityType::new(&table_name);
                let entities = graph.list_by_type(&entity_type);

                if entities.is_empty() {
                    return Err(QueryError::unknown_entity_type(&table_name));
                }

                let alias = alias
                    .as_ref()
                    .map(|a| a.name.value.clone())
                    .unwrap_or(table_name);

                Ok(entities
                    .iter()
                    .map(|entity| {
                        let mut map = HashMap::new();
                        map.insert(alias.clone(), (*entity).clone());
                        map
                    })
                    .collect())
            }
            _ => Err(QueryError::syntax("Only table references are supported", 0)),
        }
    }

    /// Apply a JOIN operation.
    fn apply_join(
        &self,
        current_entities: &[HashMap<String, Entity>],
        join_table: &TableFactor,
        join_operator: &JoinOperator,
        graph: &EntityGraph,
    ) -> QueryResult<Vec<HashMap<String, Entity>>> {
        let join_entities = self.get_table_entities(join_table, graph)?;

        match join_operator {
            JoinOperator::Inner(constraint) => {
                self.inner_join(current_entities, &join_entities, constraint, graph)
            }
            JoinOperator::LeftOuter(constraint) => {
                self.left_join(current_entities, &join_entities, constraint, graph)
            }
            JoinOperator::RightOuter(constraint) => {
                self.right_join(current_entities, &join_entities, constraint, graph)
            }
            JoinOperator::FullOuter(constraint) => {
                self.full_join(current_entities, &join_entities, constraint, graph)
            }
            JoinOperator::CrossJoin => Ok(self.cross_join(current_entities, &join_entities)),
            _ => Err(QueryError::syntax("Unsupported join type", 0)),
        }
    }

    /// Perform an INNER JOIN.
    fn inner_join(
        &self,
        left_entities: &[HashMap<String, Entity>],
        right_entities: &[HashMap<String, Entity>],
        constraint: &JoinConstraint,
        graph: &EntityGraph,
    ) -> QueryResult<Vec<HashMap<String, Entity>>> {
        let mut result = Vec::new();

        for left_map in left_entities {
            for right_map in right_entities {
                if self.evaluate_join_constraint(left_map, right_map, constraint, graph)? {
                    let mut combined_map = left_map.clone();
                    combined_map.extend(right_map.clone());
                    result.push(combined_map);
                }
            }
        }

        Ok(result)
    }

    /// Perform a LEFT JOIN.
    fn left_join(
        &self,
        left_entities: &[HashMap<String, Entity>],
        right_entities: &[HashMap<String, Entity>],
        constraint: &JoinConstraint,
        graph: &EntityGraph,
    ) -> QueryResult<Vec<HashMap<String, Entity>>> {
        let mut result = Vec::new();

        for left_map in left_entities {
            let mut found_match = false;
            for right_map in right_entities {
                if self.evaluate_join_constraint(left_map, right_map, constraint, graph)? {
                    let mut combined_map = left_map.clone();
                    combined_map.extend(right_map.clone());
                    result.push(combined_map);
                    found_match = true;
                }
            }
            if !found_match {
                result.push(left_map.clone());
            }
        }

        Ok(result)
    }

    /// Perform a RIGHT JOIN.
    fn right_join(
        &self,
        left_entities: &[HashMap<String, Entity>],
        right_entities: &[HashMap<String, Entity>],
        constraint: &JoinConstraint,
        graph: &EntityGraph,
    ) -> QueryResult<Vec<HashMap<String, Entity>>> {
        self.left_join(right_entities, left_entities, constraint, graph)
    }

    /// Perform a FULL OUTER JOIN.
    fn full_join(
        &self,
        left_entities: &[HashMap<String, Entity>],
        right_entities: &[HashMap<String, Entity>],
        constraint: &JoinConstraint,
        graph: &EntityGraph,
    ) -> QueryResult<Vec<HashMap<String, Entity>>> {
        let mut result = Vec::new();
        let mut matched_right = vec![false; right_entities.len()];

        for left_map in left_entities {
            let mut found_match = false;
            for (right_idx, right_map) in right_entities.iter().enumerate() {
                if self.evaluate_join_constraint(left_map, right_map, constraint, graph)? {
                    let mut combined_map = left_map.clone();
                    combined_map.extend(right_map.clone());
                    result.push(combined_map);
                    matched_right[right_idx] = true;
                    found_match = true;
                }
            }
            if !found_match {
                result.push(left_map.clone());
            }
        }

        for (right_idx, right_map) in right_entities.iter().enumerate() {
            if !matched_right[right_idx] {
                result.push(right_map.clone());
            }
        }

        Ok(result)
    }

    /// Perform a CROSS JOIN.
    fn cross_join(
        &self,
        left_entities: &[HashMap<String, Entity>],
        right_entities: &[HashMap<String, Entity>],
    ) -> Vec<HashMap<String, Entity>> {
        let mut result = Vec::new();

        for left_map in left_entities {
            for right_map in right_entities {
                let mut combined_map = left_map.clone();
                combined_map.extend(right_map.clone());
                result.push(combined_map);
            }
        }

        result
    }

    /// Evaluate a JOIN constraint.
    fn evaluate_join_constraint(
        &self,
        left_map: &HashMap<String, Entity>,
        right_map: &HashMap<String, Entity>,
        constraint: &JoinConstraint,
        graph: &EntityGraph,
    ) -> QueryResult<bool> {
        match constraint {
            JoinConstraint::On(expr) => {
                let mut combined_map = left_map.clone();
                combined_map.extend(right_map.clone());
                self.evaluate_expression(expr, &combined_map, graph)
                    .map(|v| self.is_truthy(&v))
            }
            JoinConstraint::Using(columns) => {
                for column_name in columns {
                    let left_value =
                        self.get_field_value_from_entities(&column_name.value, left_map, graph)?;
                    let right_value =
                        self.get_field_value_from_entities(&column_name.value, right_map, graph)?;
                    if self.compare_field_values(&left_value, &right_value)
                        != Some(std::cmp::Ordering::Equal)
                    {
                        return Ok(false);
                    }
                }
                Ok(true)
            }
            JoinConstraint::Natural => Ok(true),
            JoinConstraint::None => Ok(true),
        }
    }

    /// Filter entities based on WHERE clause.
    fn filter_entities(
        &self,
        entities: &[HashMap<String, Entity>],
        condition: &Expr,
        graph: &EntityGraph,
    ) -> QueryResult<Vec<HashMap<String, Entity>>> {
        let mut result = Vec::new();

        for entity_map in entities {
            let value = self.evaluate_expression(condition, entity_map, graph)?;
            if self.is_truthy(&value) {
                result.push(entity_map.clone());
            }
        }

        Ok(result)
    }

    /// Sort entities based on ORDER BY clause.
    fn sort_entities(
        &self,
        mut entities: Vec<HashMap<String, Entity>>,
        order_by: &[OrderByExpr],
        graph: &EntityGraph,
    ) -> QueryResult<Vec<HashMap<String, Entity>>> {
        entities.sort_by(|a, b| {
            for order_expr in order_by {
                let val_a = match self.evaluate_expression(&order_expr.expr, a, graph) {
                    Ok(v) => v,
                    Err(_) => continue,
                };
                let val_b = match self.evaluate_expression(&order_expr.expr, b, graph) {
                    Ok(v) => v,
                    Err(_) => continue,
                };

                let cmp = self.compare_field_values(&val_a, &val_b);
                if let Some(ordering) = cmp {
                    let result = if order_expr.asc.unwrap_or(true) {
                        ordering
                    } else {
                        ordering.reverse()
                    };
                    if result != std::cmp::Ordering::Equal {
                        return result;
                    }
                }
            }
            std::cmp::Ordering::Equal
        });

        Ok(entities)
    }

    /// Build column metadata from projection.

    fn build_columns_with_wildcards(
        &self,
        projection: &[SelectItem],
        entities: &[HashMap<String, Entity>],
    ) -> QueryResult<Vec<String>> {
        let mut columns = Vec::new();

        for item in projection {
            match item {
                SelectItem::UnnamedExpr(expr) => {
                    columns.push(self.expr_to_column_name(expr));
                }
                SelectItem::ExprWithAlias { alias, .. } => {
                    columns.push(alias.value.clone());
                }
                SelectItem::Wildcard(_) => {
                    let mut field_names = std::collections::BTreeSet::new();
                    for entity_map in entities {
                        for entity in entity_map.values() {
                            field_names.insert("id".to_string());
                            for (field_id, _) in &entity.fields {
                                field_names.insert(field_id.as_str().to_string());
                            }
                        }
                    }
                    columns.extend(field_names);
                }
                SelectItem::QualifiedWildcard(name, _) => {
                    let table_name = self.object_name_to_string(name);
                    let mut field_names = std::collections::BTreeSet::new();
                    for entity_map in entities {
                        if let Some(entity) = entity_map.get(&table_name) {
                            field_names.insert(format!("{}.id", table_name));
                            for (field_id, _) in &entity.fields {
                                field_names.insert(format!("{}.{}", table_name, field_id.as_str()));
                            }
                        }
                    }
                    columns.extend(field_names);
                }
            }
        }

        Ok(columns)
    }

    fn extract_row_values(
        &self,
        projection: &[SelectItem],
        entity_map: &HashMap<String, Entity>,
        graph: &EntityGraph,
        columns: &[String],
    ) -> QueryResult<Vec<FieldValue>> {
        let mut values = Vec::new();

        for item in projection {
            match item {
                SelectItem::UnnamedExpr(expr) => {
                    let value = self.evaluate_expression(expr, entity_map, graph)?;
                    values.push(value);
                }
                SelectItem::ExprWithAlias { expr, .. } => {
                    let value = self.evaluate_expression(expr, entity_map, graph)?;
                    values.push(value);
                }
                SelectItem::Wildcard(_) => {
                    for column_name in columns {
                        let mut found_value = None;
                        for entity in entity_map.values() {
                            if column_name == "id" {
                                found_value =
                                    Some(FieldValue::String(entity.id.as_str().to_string()));
                                break;
                            } else if let Some(value) = entity.get_field(&FieldId::new(column_name))
                            {
                                found_value = Some(value.clone());
                                break;
                            }
                        }
                        values.push(found_value.unwrap_or(FieldValue::String("NULL".to_string())));
                    }
                }
                SelectItem::QualifiedWildcard(name, _) => {
                    let table_name = self.object_name_to_string(name);

                    for column_name in columns {
                        if column_name.starts_with(&format!("{}.", table_name)) {
                            let field_name = column_name
                                .strip_prefix(&format!("{}.", table_name))
                                .unwrap();
                            if let Some(entity) = entity_map.get(&table_name) {
                                if field_name == "id" {
                                    values.push(FieldValue::String(entity.id.as_str().to_string()));
                                } else if let Some(value) =
                                    entity.get_field(&FieldId::new(field_name))
                                {
                                    values.push(value.clone());
                                } else {
                                    values.push(FieldValue::String("NULL".to_string()));
                                }
                            } else {
                                values.push(FieldValue::String("NULL".to_string()));
                            }
                        }
                    }
                }
            }
        }

        Ok(values)
    }

    /// Evaluate an SQL expression.
    fn evaluate_expression(
        &self,
        expr: &Expr,
        entity_map: &HashMap<String, Entity>,
        graph: &EntityGraph,
    ) -> QueryResult<FieldValue> {
        match expr {
            Expr::Identifier(ident) => {
                self.get_field_value_from_entities(&ident.value, entity_map, graph)
            }
            Expr::CompoundIdentifier(idents) => {
                let field_path = idents.iter().map(|i| i.value.as_str()).collect::<Vec<_>>();
                self.get_compound_field_value(&field_path, entity_map, graph)
            }
            Expr::Value(value) => self.convert_sql_value_to_field_value(value),
            Expr::BinaryOp { left, op, right } => {
                let left_val = self.evaluate_expression(left, entity_map, graph)?;
                let right_val = self.evaluate_expression(right, entity_map, graph)?;
                self.apply_binary_operator(&left_val, op, &right_val)
            }
            Expr::UnaryOp { op, expr } => {
                let operand = self.evaluate_expression(expr, entity_map, graph)?;
                self.apply_unary_operator(op, &operand)
            }
            Expr::Function(func) => self.evaluate_function(func, entity_map, graph),
            Expr::Nested(expr) => self.evaluate_expression(expr, entity_map, graph),
            Expr::IsNull(expr) => {
                let value = self.evaluate_expression(expr, entity_map, graph)?;
                Ok(FieldValue::Boolean(
                    matches!(value, FieldValue::String(s) if s == "NULL"),
                ))
            }
            Expr::IsNotNull(expr) => {
                let value = self.evaluate_expression(expr, entity_map, graph)?;
                Ok(FieldValue::Boolean(
                    !matches!(value, FieldValue::String(s) if s == "NULL"),
                ))
            }
            Expr::InList {
                expr,
                list,
                negated,
            } => {
                let target_value = self.evaluate_expression(expr, entity_map, graph)?;
                let mut found = false;
                for list_expr in list {
                    let list_value = self.evaluate_expression(list_expr, entity_map, graph)?;
                    if self.compare_field_values(&target_value, &list_value)
                        == Some(std::cmp::Ordering::Equal)
                    {
                        found = true;
                        break;
                    }
                }
                Ok(FieldValue::Boolean(if *negated { !found } else { found }))
            }
            Expr::Like {
                expr,
                pattern,
                negated,
                ..
            } => {
                let text_value = self.evaluate_expression(expr, entity_map, graph)?;
                let pattern_value = self.evaluate_expression(pattern, entity_map, graph)?;
                let matches = self.like_pattern_matches(&text_value, &pattern_value, false)?;
                Ok(FieldValue::Boolean(if *negated {
                    !matches
                } else {
                    matches
                }))
            }
            Expr::ILike {
                expr,
                pattern,
                negated,
                ..
            } => {
                let text_value = self.evaluate_expression(expr, entity_map, graph)?;
                let pattern_value = self.evaluate_expression(pattern, entity_map, graph)?;
                let matches = self.like_pattern_matches(&text_value, &pattern_value, true)?;
                Ok(FieldValue::Boolean(if *negated {
                    !matches
                } else {
                    matches
                }))
            }
            _ => Err(QueryError::syntax(
                &format!("Unsupported expression: {:?}", expr),
                0,
            )),
        }
    }

    /// Get field value, handling compound identifiers for reference traversal.
    fn get_compound_field_value(
        &self,
        field_path: &[&str],
        entity_map: &HashMap<String, Entity>,
        graph: &EntityGraph,
    ) -> QueryResult<FieldValue> {
        if field_path.len() == 2 {
            let table_name = field_path[0];
            let field_name = field_path[1];
            if let Some(entity) = entity_map.get(table_name) {
                self.get_field_value_from_entity(field_name, entity, graph)
            } else {
                self.get_field_value_from_entities(field_name, entity_map, graph)
            }
        } else if field_path.len() > 2 {
            let table_name = field_path[0];
            if let Some(entity) = entity_map.get(table_name) {
                let mut current_entity = entity.clone();

                for i in 1..field_path.len() - 1 {
                    let ref_field_name = field_path[i];
                    match current_entity.get_field(&FieldId::new(ref_field_name)) {
                        Some(FieldValue::Reference(ref_val)) => {
                            let entity_id = match ref_val {
                                firm_core::field::ReferenceValue::Entity(id) => id,
                                firm_core::field::ReferenceValue::Field(id, _) => id,
                            };
                            if let Some(referenced_entity) = graph.get_entity(entity_id) {
                                current_entity = referenced_entity.clone();
                            } else {
                                return Ok(FieldValue::String("NULL".to_string()));
                            }
                        }
                        _ => return Ok(FieldValue::String("NULL".to_string())),
                    }
                }

                let final_field = field_path[field_path.len() - 1];
                self.get_field_value_from_entity(final_field, &current_entity, graph)
            } else {
                Err(QueryError::unknown_entity_type(table_name))
            }
        } else {
            self.get_field_value_from_entities(field_path[0], entity_map, graph)
        }
    }

    /// Helper methods from original engine...
    fn extract_limit_value(&self, expr: &Expr) -> QueryResult<u64> {
        match expr {
            Expr::Value(Value::Number(n, _)) => n
                .parse::<u64>()
                .map_err(|_| QueryError::syntax("Invalid LIMIT/OFFSET value", 0)),
            _ => Err(QueryError::syntax("LIMIT/OFFSET must be a number", 0)),
        }
    }

    fn expr_to_column_name(&self, expr: &Expr) -> String {
        match expr {
            Expr::Identifier(ident) => ident.value.clone(),
            Expr::CompoundIdentifier(idents) => idents
                .iter()
                .map(|i| i.value.as_str())
                .collect::<Vec<_>>()
                .join("."),
            Expr::Function(func) => {
                format!("{}(...)", self.object_name_to_string(&func.name))
            }
            _ => "expr".to_string(),
        }
    }

    fn object_name_to_string(&self, name: &ObjectName) -> String {
        name.0
            .iter()
            .map(|ident| ident.value.clone())
            .collect::<Vec<_>>()
            .join(".")
    }

    fn convert_sql_value_to_field_value(&self, value: &Value) -> QueryResult<FieldValue> {
        match value {
            Value::Number(n, _) => {
                if let Ok(int_val) = n.parse::<i64>() {
                    Ok(FieldValue::Integer(int_val))
                } else if let Ok(float_val) = n.parse::<f64>() {
                    Ok(FieldValue::Float(float_val))
                } else {
                    Err(QueryError::syntax("Invalid number format", 0))
                }
            }
            Value::SingleQuotedString(s) | Value::DoubleQuotedString(s) => {
                Ok(FieldValue::String(s.clone()))
            }
            Value::Boolean(b) => Ok(FieldValue::Boolean(*b)),
            Value::Null => Ok(FieldValue::String("NULL".to_string())),
            _ => Err(QueryError::syntax(
                &format!("Unsupported literal value: {:?}", value),
                0,
            )),
        }
    }

    /// Get field value from any entity in the entity map
    fn get_field_value_from_entities(
        &self,
        field_name: &str,
        entity_map: &HashMap<String, Entity>,
        _graph: &EntityGraph,
    ) -> QueryResult<FieldValue> {
        if field_name == "id" {
            for entity in entity_map.values() {
                return Ok(FieldValue::String(entity.id.as_str().to_string()));
            }
        }

        for entity in entity_map.values() {
            if let Some(value) = entity.get_field(&FieldId::new(field_name)) {
                return Ok(value.clone());
            }
        }
        Ok(FieldValue::String("NULL".to_string()))
    }

    fn get_field_value_from_entity(
        &self,
        field_name: &str,
        entity: &Entity,
        _graph: &EntityGraph,
    ) -> QueryResult<FieldValue> {
        if field_name == "id" {
            return Ok(FieldValue::String(entity.id.as_str().to_string()));
        }

        Ok(entity
            .get_field(&FieldId::new(field_name))
            .cloned()
            .unwrap_or(FieldValue::String("NULL".to_string())))
    }

    fn apply_binary_operator(
        &self,
        left: &FieldValue,
        op: &BinaryOperator,
        right: &FieldValue,
    ) -> QueryResult<FieldValue> {
        match op {
            BinaryOperator::Eq => Ok(FieldValue::Boolean(
                self.compare_field_values(left, right) == Some(std::cmp::Ordering::Equal),
            )),
            BinaryOperator::NotEq => Ok(FieldValue::Boolean(
                self.compare_field_values(left, right) != Some(std::cmp::Ordering::Equal),
            )),
            BinaryOperator::Lt => Ok(FieldValue::Boolean(
                self.compare_field_values(left, right) == Some(std::cmp::Ordering::Less),
            )),
            BinaryOperator::LtEq => Ok(FieldValue::Boolean(matches!(
                self.compare_field_values(left, right),
                Some(std::cmp::Ordering::Less | std::cmp::Ordering::Equal)
            ))),
            BinaryOperator::Gt => Ok(FieldValue::Boolean(
                self.compare_field_values(left, right) == Some(std::cmp::Ordering::Greater),
            )),
            BinaryOperator::GtEq => Ok(FieldValue::Boolean(matches!(
                self.compare_field_values(left, right),
                Some(std::cmp::Ordering::Greater | std::cmp::Ordering::Equal)
            ))),
            BinaryOperator::And => Ok(FieldValue::Boolean(
                self.is_truthy(left) && self.is_truthy(right),
            )),
            BinaryOperator::Or => Ok(FieldValue::Boolean(
                self.is_truthy(left) || self.is_truthy(right),
            )),
            BinaryOperator::Plus => self.arithmetic_operation(left, right, |a, b| a + b),
            BinaryOperator::Minus => self.arithmetic_operation(left, right, |a, b| a - b),
            BinaryOperator::Multiply => self.arithmetic_operation(left, right, |a, b| a * b),
            BinaryOperator::Divide => self.arithmetic_operation(left, right, |a, b| a / b),
            BinaryOperator::Modulo => self.arithmetic_operation(left, right, |a, b| a % b),
            _ => Err(QueryError::syntax(
                &format!("Unsupported binary operator: {:?}", op),
                0,
            )),
        }
    }

    fn apply_unary_operator(
        &self,
        op: &UnaryOperator,
        operand: &FieldValue,
    ) -> QueryResult<FieldValue> {
        match op {
            UnaryOperator::Not => Ok(FieldValue::Boolean(!self.is_truthy(operand))),
            UnaryOperator::Plus => Ok(operand.clone()),
            UnaryOperator::Minus => match operand {
                FieldValue::Integer(i) => Ok(FieldValue::Integer(-i)),
                FieldValue::Float(f) => Ok(FieldValue::Float(-f)),
                _ => Err(QueryError::syntax("Cannot negate non-numeric value", 0)),
            },
            _ => Err(QueryError::syntax(
                &format!("Unsupported unary operator: {:?}", op),
                0,
            )),
        }
    }

    fn evaluate_function(
        &self,
        func: &Function,
        entity_map: &HashMap<String, Entity>,
        graph: &EntityGraph,
    ) -> QueryResult<FieldValue> {
        let function_name = self.object_name_to_string(&func.name).to_uppercase();

        match function_name.as_str() {
            "COUNT" => Ok(FieldValue::Integer(entity_map.len() as i64)),
            "MAX" => {
                if let FunctionArguments::List(ref arg_list) = func.args {
                    if arg_list.args.len() != 1 {
                        return Err(QueryError::syntax(
                            "MAX function requires exactly one argument",
                            0,
                        ));
                    }
                    let mut max_val: Option<FieldValue> = None;
                    for entity in entity_map.values() {
                        let val = match &arg_list.args[0] {
                            FunctionArg::Unnamed(FunctionArgExpr::Expr(expr)) => self
                                .evaluate_expression(
                                    expr,
                                    &std::iter::once((String::new(), entity.clone())).collect(),
                                    graph,
                                )?,
                            _ => {
                                return Err(QueryError::syntax("Unsupported function argument", 0))
                            }
                        };
                        match &max_val {
                            None => max_val = Some(val),
                            Some(current_max) => {
                                if self
                                    .compare_field_values(&val, current_max)
                                    .unwrap_or(std::cmp::Ordering::Equal)
                                    == std::cmp::Ordering::Greater
                                {
                                    max_val = Some(val);
                                }
                            }
                        }
                    }
                    max_val.ok_or_else(|| QueryError::syntax("No values to compute MAX", 0))
                } else {
                    Err(QueryError::syntax("MAX function requires arguments", 0))
                }
            }
            "MIN" => {
                if let FunctionArguments::List(ref arg_list) = func.args {
                    if arg_list.args.len() != 1 {
                        return Err(QueryError::syntax(
                            "MIN function requires exactly one argument",
                            0,
                        ));
                    }
                    let mut min_val: Option<FieldValue> = None;
                    for entity in entity_map.values() {
                        let val = match &arg_list.args[0] {
                            FunctionArg::Unnamed(FunctionArgExpr::Expr(expr)) => self
                                .evaluate_expression(
                                    expr,
                                    &std::iter::once((String::new(), entity.clone())).collect(),
                                    graph,
                                )?,
                            _ => {
                                return Err(QueryError::syntax("Unsupported function argument", 0))
                            }
                        };
                        match &min_val {
                            None => min_val = Some(val),
                            Some(current_min) => {
                                if self
                                    .compare_field_values(&val, current_min)
                                    .unwrap_or(std::cmp::Ordering::Equal)
                                    == std::cmp::Ordering::Less
                                {
                                    min_val = Some(val);
                                }
                            }
                        }
                    }
                    min_val.ok_or_else(|| QueryError::syntax("No values to compute MIN", 0))
                } else {
                    Err(QueryError::syntax("MIN function requires arguments", 0))
                }
            }
            "SUM" => {
                if let FunctionArguments::List(ref arg_list) = func.args {
                    if arg_list.args.len() != 1 {
                        return Err(QueryError::syntax(
                            "SUM function requires exactly one argument",
                            0,
                        ));
                    }
                    let mut sum = 0.0;
                    for entity in entity_map.values() {
                        let val = match &arg_list.args[0] {
                            FunctionArg::Unnamed(FunctionArgExpr::Expr(expr)) => self
                                .evaluate_expression(
                                    expr,
                                    &std::iter::once((String::new(), entity.clone())).collect(),
                                    graph,
                                )?,
                            _ => {
                                return Err(QueryError::syntax("Unsupported function argument", 0))
                            }
                        };
                        sum += self.to_number(&val)?;
                    }
                    Ok(FieldValue::Float(sum))
                } else {
                    Err(QueryError::syntax("SUM function requires arguments", 0))
                }
            }
            "AVG" => {
                if let FunctionArguments::List(ref arg_list) = func.args {
                    if arg_list.args.len() != 1 {
                        return Err(QueryError::syntax(
                            "AVG function requires exactly one argument",
                            0,
                        ));
                    }
                    let mut sum = 0.0;
                    let count = entity_map.len() as f64;
                    if count == 0.0 {
                        return Err(QueryError::syntax("No values to compute AVG", 0));
                    }
                    for entity in entity_map.values() {
                        let val = match &arg_list.args[0] {
                            FunctionArg::Unnamed(FunctionArgExpr::Expr(expr)) => self
                                .evaluate_expression(
                                    expr,
                                    &std::iter::once((String::new(), entity.clone())).collect(),
                                    graph,
                                )?,
                            _ => {
                                return Err(QueryError::syntax("Unsupported function argument", 0))
                            }
                        };
                        sum += self.to_number(&val)?;
                    }
                    Ok(FieldValue::Float(sum / count))
                } else {
                    Err(QueryError::syntax("AVG function requires arguments", 0))
                }
            }
            _ => Err(QueryError::syntax(
                &format!("Unsupported function: {}", function_name),
                0,
            )),
        }
    }

    fn arithmetic_operation<F>(
        &self,
        left: &FieldValue,
        right: &FieldValue,
        op: F,
    ) -> QueryResult<FieldValue>
    where
        F: Fn(f64, f64) -> f64,
    {
        let left_num = self.to_number(left)?;
        let right_num = self.to_number(right)?;
        let result = op(left_num, right_num);

        if matches!(left, FieldValue::Integer(_))
            && matches!(right, FieldValue::Integer(_))
            && result.fract() == 0.0
        {
            Ok(FieldValue::Integer(result as i64))
        } else {
            Ok(FieldValue::Float(result))
        }
    }

    fn to_number(&self, value: &FieldValue) -> QueryResult<f64> {
        match value {
            FieldValue::Integer(i) => Ok(*i as f64),
            FieldValue::Float(f) => Ok(*f),
            _ => Err(QueryError::syntax("Cannot convert to number", 0)),
        }
    }

    fn compare_field_values(
        &self,
        left: &FieldValue,
        right: &FieldValue,
    ) -> Option<std::cmp::Ordering> {
        match (left, right) {
            (FieldValue::Integer(a), FieldValue::Integer(b)) => Some(a.cmp(b)),
            (FieldValue::Float(a), FieldValue::Float(b)) => a.partial_cmp(b),
            (FieldValue::String(a), FieldValue::String(b)) => Some(a.cmp(b)),
            (FieldValue::Boolean(a), FieldValue::Boolean(b)) => Some(a.cmp(b)),
            (FieldValue::Integer(a), FieldValue::Float(b)) => (*a as f64).partial_cmp(b),
            (FieldValue::Float(a), FieldValue::Integer(b)) => a.partial_cmp(&(*b as f64)),

            (FieldValue::Reference(ref_val), FieldValue::String(str_val)) => {
                Some(ref_val.to_string().cmp(str_val))
            }
            (FieldValue::String(str_val), FieldValue::Reference(ref_val)) => {
                Some(str_val.cmp(&ref_val.to_string()))
            }

            (FieldValue::Reference(ref1), FieldValue::Reference(ref2)) => {
                Some(ref1.to_string().cmp(&ref2.to_string()))
            }

            (left, right) => match (left, right) {
                (FieldValue::List(_), _) | (_, FieldValue::List(_)) => None,
                _ => {
                    let left_str = left.to_string();
                    let right_str = right.to_string();
                    Some(left_str.cmp(&right_str))
                }
            },
        }
    }

    fn is_truthy(&self, value: &FieldValue) -> bool {
        match value {
            FieldValue::Boolean(b) => *b,
            FieldValue::Integer(i) => *i != 0,
            FieldValue::Float(f) => *f != 0.0,
            FieldValue::String(s) if s == "NULL" => false,
            FieldValue::String(s) => !s.is_empty(),
            _ => false,
        }
    }

    fn like_pattern_matches(
        &self,
        text: &FieldValue,
        pattern: &FieldValue,
        case_insensitive: bool,
    ) -> QueryResult<bool> {
        let text_str = match text {
            FieldValue::String(s) => s.clone(),
            FieldValue::Reference(r) => r.to_string(),
            FieldValue::Integer(i) => i.to_string(),
            FieldValue::Float(f) => f.to_string(),
            FieldValue::Boolean(b) => b.to_string(),
            FieldValue::DateTime(dt) => dt.to_string(),
            FieldValue::Path(p) => p.display().to_string(),
            FieldValue::Currency { amount, currency } => format!("{} {}", amount, currency),
            FieldValue::List(_) => {
                return Err(QueryError::syntax(
                    "LIKE pattern matching not supported for List fields",
                    0,
                ))
            }
        };

        let pattern_str = match pattern {
            FieldValue::String(s) => s.clone(),
            FieldValue::Reference(r) => r.to_string(),
            FieldValue::Integer(i) => i.to_string(),
            FieldValue::Float(f) => f.to_string(),
            FieldValue::Boolean(b) => b.to_string(),
            FieldValue::DateTime(dt) => dt.to_string(),
            FieldValue::Path(p) => p.display().to_string(),
            FieldValue::Currency { amount, currency } => format!("{} {}", amount, currency),
            FieldValue::List(_) => {
                return Err(QueryError::syntax(
                    "LIKE pattern matching not supported for List fields",
                    0,
                ))
            }
        };

        let text_to_match = if case_insensitive {
            text_str.to_lowercase()
        } else {
            text_str
        };
        let pattern_to_match = if case_insensitive {
            pattern_str.to_lowercase()
        } else {
            pattern_str
        };

        if pattern_to_match.contains('%') {
            let parts: Vec<&str> = pattern_to_match.split('%').collect();

            if parts.len() == 2 {
                if parts[0].is_empty() {
                    Ok(text_to_match.ends_with(parts[1]))
                } else if parts[1].is_empty() {
                    Ok(text_to_match.starts_with(parts[0]))
                } else {
                    Ok(text_to_match.starts_with(parts[0]) && text_to_match.ends_with(parts[1]))
                }
            } else {
                let mut current_pos = 0;
                let text_chars: Vec<char> = text_to_match.chars().collect();

                for (i, part) in parts.iter().enumerate() {
                    if part.is_empty() {}

                    let part_chars: Vec<char> = part.chars().collect();

                    if i == 0 {
                        if text_chars.len() < part_chars.len()
                            || text_chars[0..part_chars.len()] != part_chars[..]
                        {
                            return Ok(false);
                        }
                        current_pos = part_chars.len();
                    } else if i == parts.len() - 1 {
                        if text_chars.len() < current_pos + part_chars.len()
                            || text_chars[text_chars.len() - part_chars.len()..] != part_chars[..]
                        {
                            return Ok(false);
                        }
                    } else {
                        let remaining_text: String = text_chars[current_pos..].iter().collect();
                        if let Some(pos) =
                            remaining_text.find(&part_chars.iter().collect::<String>())
                        {
                            current_pos += pos + part_chars.len();
                        } else {
                            return Ok(false);
                        }
                    }
                }
                Ok(true)
            }
        } else {
            Ok(text_to_match == pattern_to_match)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use firm_core::{Entity, EntityId, EntityType, FieldId};

    fn create_test_graph() -> EntityGraph {
        let mut graph = EntityGraph::new();

        let person1 = Entity::new(EntityId::new("person.john"), EntityType::new("person"))
            .with_field(
                FieldId::new("name"),
                FieldValue::String("John Doe".to_string()),
            )
            .with_field(FieldId::new("age"), FieldValue::Integer(30));

        let person2 = Entity::new(EntityId::new("person.jane"), EntityType::new("person"))
            .with_field(
                FieldId::new("name"),
                FieldValue::String("Jane Smith".to_string()),
            )
            .with_field(FieldId::new("age"), FieldValue::Integer(25));

        graph.add_entity(person1).unwrap();
        graph.add_entity(person2).unwrap();
        graph.build();
        graph
    }

    #[test]
    fn test_execute_simple_select() {
        let graph = create_test_graph();
        let engine = QueryEngine::new();
        let query = sqlparser::parser::Parser::parse_sql(
            &sqlparser::dialect::GenericDialect {},
            "SELECT * FROM person",
        )
        .unwrap();

        if let sqlparser::ast::Statement::Query(q) = &query[0] {
            let result = engine.execute(q, &graph);
            assert!(result.is_ok());
        }
    }
}
