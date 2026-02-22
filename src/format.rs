//! Output format types shared across modules.

#[derive(Clone, clap::ValueEnum)]
pub enum OutputFormat {
    /// Human-readable colored output
    Pretty,
    /// JSON lines (one JSON object per line)
    Json,
}
