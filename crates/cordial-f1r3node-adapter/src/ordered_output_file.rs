//! File-based access seam for finalized ordered output.
//!
//! This module is intentionally small: it lets sidecar tooling write the
//! latest [`OrderedFinalizedOutput`] as JSON without coupling consumers to
//! the live mirror internals.

use std::path::Path;

use crate::ordered_output::OrderedFinalizedOutput;
use crate::shared_ordered_output::ReadOrderedOutput;

#[derive(Debug)]
pub enum OrderedOutputFileError {
    EmptyOutput,
    Io(std::io::Error),
    Serialize(serde_json::Error),
}

impl std::fmt::Display for OrderedOutputFileError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::EmptyOutput => write!(f, "no finalized ordered output is available to write"),
            Self::Io(err) => write!(f, "failed to write ordered output file: {err}"),
            Self::Serialize(err) => write!(f, "failed to serialize ordered output: {err}"),
        }
    }
}

impl std::error::Error for OrderedOutputFileError {}

impl From<std::io::Error> for OrderedOutputFileError {
    fn from(err: std::io::Error) -> Self {
        Self::Io(err)
    }
}

impl From<serde_json::Error> for OrderedOutputFileError {
    fn from(err: serde_json::Error) -> Self {
        Self::Serialize(err)
    }
}

/// Write a specific ordered output value to `path` as pretty JSON.
pub fn write_ordered_output_file(
    path: impl AsRef<Path>,
    output: &OrderedFinalizedOutput,
) -> Result<(), OrderedOutputFileError> {
    let json = serde_json::to_string_pretty(output)?;
    std::fs::write(path, json)?;
    Ok(())
}

/// Write the latest output from a read-only ordered-output reader.
///
/// Empty readers, or readers whose latest output has no ordered blocks, return
/// [`OrderedOutputFileError::EmptyOutput`] unless `allow_empty` is set.
pub fn write_latest_ordered_output_file(
    path: impl AsRef<Path>,
    reader: &impl ReadOrderedOutput,
    allow_empty: bool,
) -> Result<(), OrderedOutputFileError> {
    let Some(output) = reader.latest() else {
        return Err(OrderedOutputFileError::EmptyOutput);
    };

    if output.is_empty() && !allow_empty {
        return Err(OrderedOutputFileError::EmptyOutput);
    }

    write_ordered_output_file(path, output)
}
