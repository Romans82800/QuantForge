//! Deterministic, guarded Strategy IR to MetaTrader 5 code generation.

mod compiler;
mod generator;
mod model;
mod runner;

pub use compiler::compile_with_metaeditor;
pub use generator::generate_bundle;
pub use model::{
    CompileReport, ExportBundle, ExportError, ExportEvidenceCard, ExportStyle, ExportSupportFile,
    MetaEditorConfig, Mql5ExportConfig, TerminalConfig, TesterConfig, TesterRunReport,
    suggested_expert_name,
};
pub use runner::run_mt5_tester;

pub const EXPORT_SCHEMA_VERSION: u16 = 1;
pub const EXPORT_TARGET: &str = "MetaTrader 5";
