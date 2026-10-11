//! Mechanical check of the operation work contracts in `.performance/contract.json`.
//!
//! The syntax and the growth semantics are those of coding-tooling `performance work`
//! (coding-tooling#300, `docs/work-complexity.md`); coding-tooling stays the authority and
//! verifies the emitted evidence again in CI. This copy lets the blocking performance smoke
//! fail on its own and fail closed: a contract that is malformed or incomplete, or a
//! measurement that misses a declared metric, is an error and never a pass.

use std::collections::{BTreeMap, BTreeSet};

use serde_json::{Map, Value, json};

/// Values of the declared scale dimensions at one scale point.
pub type Dimensions = BTreeMap<String, u64>;
/// Counter values measured at one scale point.
pub type Metrics = BTreeMap<String, u64>;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Bound {
    Constant,
    Linear,
}

impl Bound {
    fn name(self) -> &'static str {
        match self {
            Self::Constant => "constant",
            Self::Linear => "linear",
        }
    }
}

struct Growth {
    dimension: String,
    bound: Bound,
}

struct MetricSpec {
    name: String,
    budget: Option<u64>,
    growth: Vec<Growth>,
}

struct OperationSpec {
    id: String,
    scale_points: Vec<Dimensions>,
    metrics: Vec<MetricSpec>,
}

fn is_operation_id(id: &str) -> bool {
    id.bytes()
        .next()
        .is_some_and(|first| first.is_ascii_lowercase() || first.is_ascii_digit())
        && id
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
}

fn is_dimension_name(name: &str) -> bool {
    name.bytes()
        .next()
        .is_some_and(|first| first.is_ascii_alphabetic())
        && name
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_')
}

/// Groups `points` by the values of every dimension except `dimension`; each group holds
/// the indices of its points.
fn groups<'a>(
    points: &'a [Dimensions],
    dimension: &str,
) -> BTreeMap<Vec<(&'a String, &'a u64)>, Vec<usize>> {
    let mut groups = BTreeMap::<_, Vec<usize>>::new();
    for (index, point) in points.iter().enumerate() {
        let key = point
            .iter()
            .filter(|(name, _)| name.as_str() != dimension)
            .collect::<Vec<_>>();
        groups.entry(key).or_default().push(index);
    }
    groups
}

fn parse_scale_points(
    id: &str,
    operation: &Map<String, Value>,
    dimensions: &BTreeSet<String>,
    errors: &mut Vec<String>,
) -> Vec<Dimensions> {
    let Some(points) = operation
        .get("scalePoints")
        .and_then(Value::as_array)
        .filter(|points| !points.is_empty())
    else {
        errors.push(format!("{id}: scalePoints must be a non-empty array"));
        return Vec::new();
    };
    let mut parsed = Vec::new();
    for (index, point) in points.iter().enumerate() {
        let Some(point) = point.as_object() else {
            errors.push(format!("{id}: scale point {index} is not an object"));
            continue;
        };
        let names = point.keys().cloned().collect::<BTreeSet<_>>();
        if &names != dimensions {
            errors.push(format!(
                "{id}: scale point {index} names {names:?}, the declared dimensions are {dimensions:?}"
            ));
            continue;
        }
        let mut values = Dimensions::new();
        for (name, value) in point {
            match value.as_u64().filter(|&value| value > 0) {
                Some(value) => {
                    values.insert(name.clone(), value);
                }
                None => errors.push(format!(
                    "{id}: scale point {index} gives {name} a value that is not a positive integer"
                )),
            }
        }
        if values.len() != point.len() {
            continue;
        }
        if parsed.contains(&values) {
            errors.push(format!("{id}: scale point {index} repeats {values:?}"));
            continue;
        }
        parsed.push(values);
    }
    parsed
}

fn parse_metric(
    id: &str,
    index: usize,
    metric: &Value,
    dimensions: &BTreeSet<String>,
    errors: &mut Vec<String>,
) -> Option<MetricSpec> {
    let Some(metric) = metric.as_object() else {
        errors.push(format!("{id}: metric {index} is not an object"));
        return None;
    };
    let Some(name) = metric
        .get("name")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|name| !name.is_empty())
    else {
        errors.push(format!("{id}: metric {index} has no name"));
        return None;
    };
    let before = errors.len();
    let budget = match metric.get("budget") {
        None => None,
        Some(budget) => {
            let max = budget.get("max").and_then(Value::as_u64);
            if max.is_none() || budget.as_object().is_none_or(|budget| budget.len() != 1) {
                errors.push(format!(
                    "{id}: {name}: budget must be {{ \"max\": <non-negative integer> }}"
                ));
            }
            max
        }
    };
    let mut growth = Vec::new();
    match metric.get("growth") {
        None => {}
        Some(Value::Array(entries)) => {
            for entry in entries {
                let dimension = entry.get("dimension").and_then(Value::as_str);
                let bound = match entry.get("bound").and_then(Value::as_str) {
                    Some("constant") => Some(Bound::Constant),
                    Some("linear") => Some(Bound::Linear),
                    _ => None,
                };
                match (dimension, bound) {
                    (Some(dimension), Some(bound))
                        if dimensions.contains(dimension) && entry.as_object().is_some_and(|entry| entry.len() == 2) =>
                    {
                        if growth.iter().any(|known: &Growth| known.dimension == dimension) {
                            errors.push(format!(
                                "{id}: {name}: repeats a growth bound for {dimension}"
                            ));
                        }
                        growth.push(Growth {
                            dimension: dimension.to_owned(),
                            bound,
                        });
                    }
                    _ => errors.push(format!(
                        "{id}: {name}: growth entry {entry} needs a declared dimension and a bound of constant or linear"
                    )),
                }
            }
        }
        Some(_) => errors.push(format!("{id}: {name}: growth must be an array")),
    }
    if budget.is_none() && growth.is_empty() && errors.len() == before {
        errors.push(format!(
            "{id}: {name}: declares neither a budget nor a growth bound"
        ));
    }
    (errors.len() == before).then(|| MetricSpec {
        name: name.to_owned(),
        budget,
        growth,
    })
}

fn parse_operation(operation: &Value, errors: &mut Vec<String>) -> Option<OperationSpec> {
    let Some(operation) = operation.as_object() else {
        errors.push("an operation is not an object".to_owned());
        return None;
    };
    let Some(id) = operation
        .get("id")
        .and_then(Value::as_str)
        .filter(|id| is_operation_id(id))
    else {
        errors.push("an operation has no valid id".to_owned());
        return None;
    };
    let before = errors.len();
    let dimensions = match operation.get("dimensions").and_then(Value::as_object) {
        Some(dimensions) if !dimensions.is_empty() => {
            for name in dimensions.keys().filter(|name| !is_dimension_name(name)) {
                errors.push(format!("{id}: dimension name {name:?} is invalid"));
            }
            dimensions.keys().cloned().collect::<BTreeSet<_>>()
        }
        _ => {
            errors.push(format!("{id}: dimensions must be a non-empty object"));
            BTreeSet::new()
        }
    };
    let scale_points = parse_scale_points(id, operation, &dimensions, errors);
    let mut metrics = Vec::new();
    match operation
        .get("metrics")
        .and_then(Value::as_array)
        .filter(|metrics| !metrics.is_empty())
    {
        Some(declared) => {
            for (index, metric) in declared.iter().enumerate() {
                if let Some(metric) = parse_metric(id, index, metric, &dimensions, errors) {
                    if metrics
                        .iter()
                        .any(|known: &MetricSpec| known.name == metric.name)
                    {
                        errors.push(format!("{id}: metric {} is declared twice", metric.name));
                    }
                    metrics.push(metric);
                }
            }
        }
        None => errors.push(format!("{id}: metrics must be a non-empty array")),
    }
    // A growth bound needs some group of points that varies its dimension.
    for metric in &metrics {
        for growth in &metric.growth {
            if groups(&scale_points, &growth.dimension)
                .values()
                .all(|group| group.len() < 2)
            {
                errors.push(format!(
                    "{id}: {}: no scale points vary {} with the other dimensions fixed",
                    metric.name, growth.dimension
                ));
            }
        }
    }
    (errors.len() == before).then(|| OperationSpec {
        id: id.to_owned(),
        scale_points,
        metrics,
    })
}

fn parse(contract: &str) -> Result<Vec<OperationSpec>, Vec<String>> {
    let contract: Value =
        serde_json::from_str(contract).map_err(|error| vec![format!("contract: {error}")])?;
    if contract.get("schemaVersion") != Some(&json!(2)) {
        return Err(vec!["contract is not schemaVersion 2".to_owned()]);
    }
    let declared = contract
        .get("operations")
        .and_then(Value::as_array)
        .filter(|operations| !operations.is_empty())
        .ok_or_else(|| vec!["contract declares no operations".to_owned()])?;
    let mut errors = Vec::new();
    let mut operations = Vec::<OperationSpec>::new();
    for operation in declared {
        if let Some(operation) = parse_operation(operation, &mut errors) {
            if operations.iter().any(|known| known.id == operation.id) {
                errors.push(format!("{}: operation is declared twice", operation.id));
            }
            operations.push(operation);
        }
    }
    if errors.is_empty() {
        Ok(operations)
    } else {
        Err(errors)
    }
}

/// The coding-tooling growth semantics over the points of one group, ordered by the bounded
/// dimension: `constant` keeps every value at most the first; `linear` keeps every later
/// interval's marginal rate at most the first interval's, or with two points applies the
/// ratio test.
fn growth_holds(bound: Bound, points: &[(i128, i128)], index: usize) -> bool {
    let (x0, v0) = points[0];
    let (x1, v1) = points[1];
    let (x, v) = points[index];
    match bound {
        Bound::Constant => v <= v0,
        Bound::Linear if points.len() == 2 => v * x0 <= v0 * x,
        Bound::Linear => {
            let (previous_x, previous_v) = points[index - 1];
            (v - previous_v) * (x1 - x0) <= (v1 - v0).max(0) * (x - previous_x)
        }
    }
}

fn check_bounds(operation: &OperationSpec, samples: &[Metrics], failures: &mut Vec<String>) {
    let id = &operation.id;
    for metric in &operation.metrics {
        let name = &metric.name;
        if let Some(max) = metric.budget {
            for (point, sample) in operation.scale_points.iter().zip(samples) {
                let value = sample[name];
                if value > max {
                    failures.push(format!(
                        "{id}: {name}={value} exceeds its budget {max} at {point:?}"
                    ));
                }
            }
        }
        for growth in &metric.growth {
            let dimension = &growth.dimension;
            for (others, group) in groups(&operation.scale_points, dimension) {
                let mut points = group
                    .iter()
                    .map(|&index| {
                        (
                            i128::from(operation.scale_points[index][dimension]),
                            i128::from(samples[index][name]),
                        )
                    })
                    .collect::<Vec<_>>();
                points.sort_unstable();
                for index in 1..points.len() {
                    if !growth_holds(growth.bound, &points, index) {
                        let ((x0, v0), (x, v)) = (points[0], points[index]);
                        failures.push(format!(
                            "{id}: {name} is not {} in {dimension} (others {others:?}): {v} at {dimension}={x}, {v0} at {dimension}={x0}",
                            growth.bound.name()
                        ));
                    }
                }
            }
        }
    }
}

/// Measures every declared operation at every declared scale point with `measure` and
/// checks the declared budgets and growth bounds. Returns the `operations` part of a
/// `coding-tooling/work-evidence/v1` document, or every reason the contract does not hold.
/// `measure` reports an operation it has no runner for as an error.
pub fn check_work_contract(
    contract: &str,
    measure: &mut dyn FnMut(&str, &Dimensions) -> Result<Metrics, String>,
) -> Result<Value, Vec<String>> {
    let operations = parse(contract)?;
    let mut failures = Vec::new();
    let mut evidence = Vec::new();
    for operation in &operations {
        let id = &operation.id;
        let declared = operation
            .metrics
            .iter()
            .map(|metric| metric.name.clone())
            .collect::<BTreeSet<_>>();
        let before = failures.len();
        let mut samples = Vec::new();
        for point in &operation.scale_points {
            match measure(id, point) {
                Ok(metrics) => {
                    let measured = metrics.keys().cloned().collect::<BTreeSet<_>>();
                    if measured != declared {
                        failures.push(format!(
                            "{id} {point:?}: measured metrics {measured:?}, declared {declared:?}"
                        ));
                    }
                    samples.push(metrics);
                }
                Err(error) => failures.push(format!("{id} {point:?}: {error}")),
            }
        }
        // Bounds are only evaluated on complete samples with exactly the declared metrics.
        if failures.len() == before {
            check_bounds(operation, &samples, &mut failures);
        }
        evidence.push(json!({
            "id": id,
            "samples": operation
                .scale_points
                .iter()
                .zip(&samples)
                .map(|(point, metrics)| json!({ "dimensions": point, "metrics": metrics }))
                .collect::<Vec<_>>(),
        }));
    }
    if failures.is_empty() {
        Ok(json!({ "operations": evidence }))
    } else {
        Err(failures)
    }
}
