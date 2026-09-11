//! Validated measurement primitives.

use serde::{Deserialize, Serialize};
use std::fmt;
use thiserror::Error;

/// Error returned when constructing an invalid measurement.
#[derive(Debug, Clone, PartialEq, Error)]
pub enum MetricError {
    /// Measurement names cannot be blank.
    #[error("measurement name cannot be empty")]
    EmptyName,
    /// Numeric values must be finite so ordering stays deterministic.
    #[error("metric value must be finite")]
    NonFiniteValue,
}

/// Finite floating-point value with total deterministic ordering.
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(try_from = "f64", into = "f64")]
pub struct FiniteValue(f64);

impl FiniteValue {
    /// Creates a finite value.
    ///
    /// # Errors
    ///
    /// Returns [`MetricError::NonFiniteValue`] for NaN or infinity.
    pub fn new(value: f64) -> Result<Self, MetricError> {
        if value.is_finite() {
            Ok(Self(value))
        } else {
            Err(MetricError::NonFiniteValue)
        }
    }

    /// Returns wrapped floating-point value.
    #[must_use]
    pub const fn get(self) -> f64 {
        self.0
    }
}

impl TryFrom<f64> for FiniteValue {
    type Error = MetricError;

    fn try_from(value: f64) -> Result<Self, Self::Error> {
        Self::new(value)
    }
}

impl From<FiniteValue> for f64 {
    fn from(value: FiniteValue) -> Self {
        value.get()
    }
}

impl PartialEq for FiniteValue {
    fn eq(&self, other: &Self) -> bool {
        self.0.total_cmp(&other.0).is_eq()
    }
}

impl Eq for FiniteValue {}

impl PartialOrd for FiniteValue {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for FiniteValue {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.0.total_cmp(&other.0)
    }
}

impl fmt::Display for FiniteValue {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(formatter)
    }
}

/// Complete set of measurement classes exposed in reports and protocols.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MetricKind {
    /// Boolean threshold that every candidate must satisfy.
    HardGate,
    /// Frozen numeric metric that controls keep or discard.
    Objective,
    /// Numeric metric used only when primary objective is equal.
    TieBreaker,
    /// Recorded numeric observation that cannot affect selection.
    Diagnostic,
    /// Imported commercial evidence that cannot affect internal selection.
    MarketEvidence,
}

/// Numeric measurement classes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NumericMetricKind {
    /// Frozen numeric metric that controls keep or discard.
    Objective,
    /// Numeric metric used only when primary objective is equal.
    TieBreaker,
    /// Recorded numeric observation that cannot affect selection.
    Diagnostic,
    /// Imported commercial evidence kept separate from capability metrics.
    MarketEvidence,
}

impl From<NumericMetricKind> for MetricKind {
    fn from(kind: NumericMetricKind) -> Self {
        match kind {
            NumericMetricKind::Objective => Self::Objective,
            NumericMetricKind::TieBreaker => Self::TieBreaker,
            NumericMetricKind::Diagnostic => Self::Diagnostic,
            NumericMetricKind::MarketEvidence => Self::MarketEvidence,
        }
    }
}

/// Direction in which a numeric metric improves.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MetricDirection {
    /// Larger values improve.
    Maximize,
    /// Smaller values improve.
    Minimize,
}

/// Result of one hard gate.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GateOutcome {
    passed: bool,
    detail: Option<String>,
}

impl GateOutcome {
    /// Creates a gate result and optional evidence detail.
    #[must_use]
    pub fn new(passed: bool, detail: Option<String>) -> Self {
        Self { passed, detail }
    }

    /// Returns whether gate passed.
    #[must_use]
    pub const fn passed(&self) -> bool {
        self.passed
    }

    /// Returns evaluator-provided gate detail.
    #[must_use]
    pub fn detail(&self) -> Option<&str> {
        self.detail.as_deref()
    }
}

/// One validated evaluator output.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Measurement {
    /// Boolean hard-gate result.
    HardGate {
        /// Stable evaluator-defined name.
        name: String,
        /// Pass state and supporting detail.
        outcome: GateOutcome,
    },
    /// Numeric measurement with explicit role and direction.
    Numeric {
        /// Stable evaluator-defined name.
        name: String,
        /// Role in evaluation and reporting.
        metric_kind: NumericMetricKind,
        /// Improvement direction frozen at baseline.
        direction: MetricDirection,
        /// Finite value.
        value: FiniteValue,
    },
}

impl Measurement {
    /// Creates a hard-gate measurement.
    ///
    /// # Errors
    ///
    /// Returns [`MetricError::EmptyName`] when `name` is blank.
    pub fn hard_gate(
        name: impl Into<String>,
        passed: bool,
        detail: Option<String>,
    ) -> Result<Self, MetricError> {
        let name = checked_name(name)?;
        Ok(Self::HardGate {
            name,
            outcome: GateOutcome::new(passed, detail),
        })
    }

    /// Creates a numeric measurement.
    ///
    /// # Errors
    ///
    /// Returns an error when `name` is blank or `value` is not finite.
    pub fn numeric(
        name: impl Into<String>,
        metric_kind: NumericMetricKind,
        direction: MetricDirection,
        value: f64,
    ) -> Result<Self, MetricError> {
        let name = checked_name(name)?;
        Ok(Self::Numeric {
            name,
            metric_kind,
            direction,
            value: FiniteValue::new(value)?,
        })
    }

    /// Returns stable measurement name.
    #[must_use]
    pub fn name(&self) -> &str {
        match self {
            Self::HardGate { name, .. } | Self::Numeric { name, .. } => name,
        }
    }

    /// Returns measurement class.
    #[must_use]
    pub fn kind(&self) -> MetricKind {
        match self {
            Self::HardGate { .. } => MetricKind::HardGate,
            Self::Numeric { metric_kind, .. } => (*metric_kind).into(),
        }
    }
}

fn checked_name(name: impl Into<String>) -> Result<String, MetricError> {
    let name = name.into();
    if name.trim().is_empty() {
        Err(MetricError::EmptyName)
    } else {
        Ok(name)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_non_finite_values() {
        assert_eq!(FiniteValue::new(f64::NAN), Err(MetricError::NonFiniteValue));
        assert!(FiniteValue::new(f64::INFINITY).is_err());
    }

    #[test]
    fn rejects_blank_names() {
        assert_eq!(
            Measurement::hard_gate("  ", true, None),
            Err(MetricError::EmptyName)
        );
    }

    #[test]
    fn signed_zero_has_deterministic_order() {
        let negative = FiniteValue::new(-0.0).expect("finite fixture");
        let positive = FiniteValue::new(0.0).expect("finite fixture");
        assert!(negative < positive);
    }
}
