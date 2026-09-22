use std::path::Path;

use eros::Context;
use tokio::fs;

use crate::build::YARD_YAML_FILE_NAME;

pub async fn init(path: &Path) -> eros::Result<()> {
    let template_file = path.join(YARD_YAML_FILE_NAME);
    let simple_template = include_str!("templates/simple/yard.yaml");
    fs::write(&template_file, simple_template)
        .await
        .with_context(|| {
            format!(
                "Write initial configuration to '{}'",
                template_file.display()
            )
        })
        .user_context("Could not create yard.yaml. Check that the destination is writable.")?;
    Ok(())
}
