use std::{
    fs::{self, File},
    io::BufReader,
    path::{Path, PathBuf},
};

use anyhow::{bail, Context};
use jsonschema::{Draft, JSONSchema};

use crate::common::UserMessageError;

pub struct Module {
    pub module: serde_yaml::Value,
    pub containerfile_path: PathBuf,
}

pub fn resolve_modules(paths: Vec<(&Path, &Path)>) -> anyhow::Result<Vec<Module>> {
    let yard_module_schema: &'static str = include_str!("./schemas/yard-module-schema.json");
    let yard_module_schema: serde_json::Value = serde_json::from_str(yard_module_schema)
        .expect("yard-module-schema.json is not valid json");
    let schema = JSONSchema::options()
        .with_draft(Draft::Draft7)
        .compile(&yard_module_schema)
        .expect("yard-module-schema.json is not a valid json schema");
    let mut modules = Vec::new();
    for (cache_dir, relative_module_dir) in paths.iter() {
        let relative_module_yaml_path = relative_module_dir.join("module.yaml");
        let module_yaml_path = cache_dir.join(relative_module_yaml_path.clone());
        if !module_yaml_path.is_file() {
            bail!(UserMessageError::new(format!(
                "'{}' is not an existing file",
                relative_module_yaml_path.display()
            )))
        }
        let module_yaml_file = File::open(module_yaml_path.clone())
            .context(format!("Could not open '{}'", module_yaml_path.display()))?;
        let yard_module_yaml: serde_yaml::Value =
            serde_yaml::from_reader(BufReader::new(module_yaml_file))
                .context("yard-module-schema.json is not valid json")?;
        let yard_module_json = serde_json::to_value(&yard_module_yaml).context(format!(
            "Could not convert the '{}' file to json for validation against the schema.",
            relative_module_yaml_path.display()
        ))?;
        schema
            .validate(&yard_module_json)
            .map_err(|errors| {
                let mut error_message = String::new();
                for error in errors {
                    error_message.push_str(&format!(
                        "Validation error: {}\nInstance path: {}\n",
                        error, error.instance_path
                    ));
                }
                UserMessageError::new(error_message)
            })
            .context(UserMessageError::new(format!(
                "'{}' does not follow the proper schema.",
                relative_module_yaml_path.display()
            )))?;
        let relative_containerfile_path = relative_module_dir.join("Containerfile");
        let containerfile_path = cache_dir.join(relative_containerfile_path.clone());
        if !containerfile_path.is_file() {
            bail!(UserMessageError::new(format!(
                "'{}' is not an existing file",
                relative_containerfile_path.display()
            )))
        }
        modules.push(Module {
            module: yard_module_yaml,
            containerfile_path: containerfile_path,
        });
    }
    return Ok(modules);
}
