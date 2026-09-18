//! Text/data engine: serde ecosystem. Structured→structured only;
//! md→PDF is intentionally simple (fixed-width text + pagination) for MVP.

use super::{ConvertError, Job};
use zest_core::Settings;

pub async fn convert(job: &Job, _settings: &Settings) -> Result<std::path::PathBuf, ConvertError> {
    // TODO(MVP-text): json/yaml/toml/xml/csv/txt/md matrix + simple md→pdf.
    let _ = job;
    Err(ConvertError::NotImplemented(
        "text engine (serde)".to_string(),
    ))
}
