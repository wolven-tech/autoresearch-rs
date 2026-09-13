//! Generic exit-code hard-gate adapter for local checks with ordinary stdout.

use crate::process::execute_capture;
use crate::{
    CancellationToken, EvaluationContext, EvaluatorOutput, FailureClass, ProcessDiagnostics,
    ProcessEvaluation, ProcessFailure, ProcessLimits, validate_output,
};
use autoresearch_config::Evaluator;
use autoresearch_core::Measurement;

/// Executes one frozen command as exactly one declared hard gate.
///
/// Raw command mode accepts arbitrary bounded stdout but never parses it into
/// a numeric score. Exit zero creates the declared passed gate. Non-zero exit
/// is an evaluator failure, not a score or passed gate. To emit numeric output,
/// use the versioned JSONL subprocess evaluator instead.
///
/// # Errors
///
/// Rejects zero/multiple gate declarations, any numeric declaration, process
/// failures, or structural output-validation failures.
pub fn evaluate_command_gate(
    context: &EvaluationContext,
    evaluator: &Evaluator,
    limits: ProcessLimits,
    cancellation: &CancellationToken,
) -> Result<ProcessEvaluation, ProcessFailure> {
    if evaluator.hard_gates().len() != 1 || !evaluator.metrics().is_empty() {
        return Err(ProcessFailure::new(
            FailureClass::Validation,
            "raw command requires exactly one hard gate and no numeric metrics",
            0,
            0,
        ));
    }
    let captured = execute_capture(context, evaluator, limits, cancellation, None)?;
    let diagnostics = ProcessDiagnostics::new(captured.stdout.len(), captured.stderr.len());
    if !captured.status.success() {
        return Err(ProcessFailure::new(
            FailureClass::NonZeroExit,
            format!("command exited with code {:?}", captured.status.code()),
            captured.stdout.len(),
            captured.stderr.len(),
        ));
    }
    let gate =
        Measurement::hard_gate(evaluator.hard_gates()[0].clone(), true, None).map_err(|_| {
            ProcessFailure::new(
                FailureClass::Validation,
                "declared hard gate is invalid",
                captured.stdout.len(),
                captured.stderr.len(),
            )
        })?;
    let output = EvaluatorOutput {
        evaluator_id: evaluator.id().to_owned(),
        run_id: context.run_id().to_string(),
        baseline_commit: context.baseline_commit().to_string(),
        evaluated_commit: context.evaluated_commit().to_string(),
        measurements: vec![gate],
        observations: vec![],
        artifacts: vec![],
        warnings: vec![],
    };
    let output = validate_output(context, evaluator.id(), output).map_err(|_| {
        ProcessFailure::new(
            FailureClass::Validation,
            "raw command output invalid",
            captured.stdout.len(),
            captured.stderr.len(),
        )
    })?;
    Ok(ProcessEvaluation {
        output,
        diagnostics,
    })
}
