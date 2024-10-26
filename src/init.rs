use std::path::Path;

use tokio::fs;

use crate::build::YARD_YAML_FILE_NAME;

pub async fn init(path: &Path) -> anyhow::Result<()> {
    let template_file = path.join(YARD_YAML_FILE_NAME);
    let simple_template = include_str!("templates/simple/yard.yaml");
    fs::write(template_file, simple_template).await?;
    Ok(())
}
