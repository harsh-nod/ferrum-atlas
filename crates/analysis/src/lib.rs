//! Deterministic, bounded interpretations over explicitly selected facts.
mod compiler_complexity;
mod dataflow;
mod graph;
mod maintainability;
mod state_machine;
mod trace;
pub use compiler_complexity::*;
pub use dataflow::*;
pub use graph::*;
pub use maintainability::*;
pub use state_machine::*;
pub use trace::*;

use atlas_model::{Coverage, ReasonCount, Status, UnknownReason};
use serde::{Deserialize, Serialize};
use std::{
    io::{self, Write},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};
use ts_rs::TS;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AnalysisError {
    InvalidInput(String),
    BudgetExhausted,
}
impl std::fmt::Display for AnalysisError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InvalidInput(message) => write!(f, "invalid analysis input: {message}"),
            Self::BudgetExhausted => f.write_str("analysis byte or hard input budget exhausted"),
        }
    }
}
impl std::error::Error for AnalysisError {}
pub type Result<T> = std::result::Result<T, AnalysisError>;

#[derive(Clone)]
pub struct AnalysisControl {
    deadline: Instant,
    cancelled: Arc<AtomicBool>,
}
impl AnalysisControl {
    pub fn new(duration: Duration) -> Self {
        Self {
            deadline: Instant::now() + duration.min(Duration::from_secs(60)),
            cancelled: Arc::new(AtomicBool::new(false)),
        }
    }
    pub fn cancel(&self) {
        self.cancelled.store(true, Ordering::Relaxed);
    }
    pub fn cancelled(&self) -> bool {
        self.cancelled.load(Ordering::Relaxed)
    }
    pub fn deadline_reached(&self) -> bool {
        Instant::now() >= self.deadline
    }
    pub fn stopped(&self) -> bool {
        self.cancelled() || self.deadline_reached()
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
pub struct AnalysisEnvelope {
    pub algorithm_version: String,
    pub coverage: Coverage,
    pub truncated: bool,
    pub cancelled: bool,
    pub deadline_reached: bool,
    pub assumptions: Vec<String>,
}
impl AnalysisEnvelope {
    fn new(version: &str, coverage: Coverage) -> Self {
        Self {
            algorithm_version: version.into(),
            coverage,
            truncated: false,
            cancelled: false,
            deadline_reached: false,
            assumptions: vec![],
        }
    }
    fn partial(&mut self, reason: UnknownReason, message: &str) {
        if self.coverage.status == Status::Complete {
            self.coverage.status = Status::Partial;
        }
        if let Some(count) = self
            .coverage
            .reasons
            .iter_mut()
            .find(|r| r.reason == reason)
        {
            count.count = count.count.saturating_add(1);
        } else {
            self.coverage.reasons.push(ReasonCount { reason, count: 1 });
        }
        if !self.coverage.limitations.iter().any(|text| text == message) {
            self.coverage.limitations.push(message.into());
        }
        self.coverage
            .reasons
            .sort_by(|a, b| a.reason.cmp(&b.reason));
    }
    fn stop(&mut self, control: &AnalysisControl) {
        self.cancelled = control.cancelled();
        self.deadline_reached = control.deadline_reached();
        self.truncated = true;
        self.partial(
            UnknownReason::BudgetExhausted,
            "Analysis stopped before all selected facts were processed",
        );
    }
}

fn byte_budget<T: Serialize + ?Sized>(value: &T, max_bytes: usize) -> Result<()> {
    struct Counter {
        remaining: usize,
    }
    impl Write for Counter {
        fn write(&mut self, data: &[u8]) -> io::Result<usize> {
            if data.len() > self.remaining {
                return Err(io::Error::other("byte budget"));
            }
            self.remaining -= data.len();
            Ok(data.len())
        }
        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }
    serde_json::to_writer(
        Counter {
            remaining: max_bytes,
        },
        value,
    )
    .map_err(|_| AnalysisError::BudgetExhausted)
}

pub fn typescript() -> String {
    let mut output = String::new();
    macro_rules! emit { ($($t:ty),+ $(,)?) => { $(output.push_str("export "); output.push_str(&<$t>::decl()); output.push('\n');)+ }; }
    emit!(
        AnalysisEnvelope,
        AnalysisLimits,
        UnknownFrontier,
        RecursiveComponent,
        ComponentLink,
        DefinitionMetrics,
        MetricDistribution,
        GraphAnalysis,
        PathOutcome,
        PathAnalysis,
        TraceCompareRequest,
        TracePosition,
        TraceDivergence,
        TraceComparison,
        DataflowLimits,
        MirPoint,
        LocalDefinition,
        LocalUse,
        UnknownMemoryEffect,
        ReachingDefinitions,
        StateMachineLimits,
        StateMachineInference,
        StateTransitionCandidate,
        StateMachineSyntax,
        StateMachineUnknown,
        StateTransitionDecision,
        StateTransitionReview,
        MaintainabilityLimits,
        SyntaxMetrics,
        NestingSite,
        UnsafeBoundary,
        MetricUnknown,
        SourceMaintainability,
        CompilerComplexityLimits,
        CfgComplexityMetrics,
        CfgExitSite,
        CompilerComplexity
    );
    output
}
