use std::{
    collections::{HashMap, HashSet},
    fmt::{Debug, Display},
    fs::{self, File},
    io::BufReader,
    path::{Path, PathBuf},
};

use anyhow::{bail, Context};
use const_format::formatcp;
use enum_dispatch::enum_dispatch;
use jsonschema::{Draft, JSONSchema};
use serde::{Deserialize, Serialize};

use crate::{
    common::UserMessageError,
    git::{git_provider_from_url, GitProvider},
};

pub const MODULE_YAML_FILE_NAME: &str = "yard-module.yaml";
pub const YARD_YAML_FILE_NAME: &str = "yard.yaml";
pub const CONTAINERFILE_NAME: &str = "Containerfile";

pub async fn build(path: &Path) -> anyhow::Result<()> {
    let parsed_yard_file = parse_yard_yaml(path)?;
    let resolved_yard_file = resolve_yard_yaml(parsed_yard_file).await?;
    let containerfile = apply_templates(resolved_yard_file)?;
    fs::write("Containerfile", containerfile)?;
    Ok(())
}

// Deserialized yard-module.yaml
//************************************************************************//
/// Created using the yard-module-schema.json file and https://app.quicktype.io/
#[derive(Debug, Clone, PartialEq, Default, Deserialize)]
pub struct YamlModule {
    pub args: Option<YamlArgs>,
    /// This is a modules description
    pub description: String,
}

#[derive(Debug, Clone, PartialEq, Default, Deserialize)]
pub struct YamlArgs {
    pub optional: Option<Vec<String>>,
    pub required: Option<Vec<String>>,
}

// Deserialized yard.yaml
//************************************************************************//

/// Created using the yard-schema.json file and https://app.quicktype.io/
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct YamlYard {
    pub inputs: YamlInputs,
    pub outputs: HashMap<String, HashMap<String, Option<YamlOutput>>>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct YamlInputs {
    pub paths: Option<HashMap<String, String>>,
    pub remotes: Option<Vec<YamlRemote>>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct YamlRemote {
    pub commit: String,
    pub paths: HashMap<String, String>,
    pub url: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum YamlOutput {
    String(String),
    StringMap(HashMap<String, String>),
}

// Intermediate  yard.yaml reprsentation
//************************************************************************//

#[derive(Debug, Clone, Default)]
struct IntermediateYardFile {
    input_remotes: Vec<IntermediateRemote>,
    /// Module name to path on local
    input_paths: HashMap<String, String>,
    /// Containerfile name to included modules
    output_container_files: HashMap<String, Vec<IntermediateUseModule>>,
}

/// Reference to a remote and containing modules
#[derive(Debug, Clone, Default)]
pub struct IntermediateRemote {
    pub url: String,
    pub commit: String,
    pub name_to_path: HashMap<String, String>,
}

/// Reference to an input module or inline
#[derive(Debug, Clone)]
enum IntermediateUseModule {
    Inline(IntermediateUseInlineModule),
    Input(IntermediateUseInputModule),
}

/// Inline module
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
struct IntermediateUseInlineModule {
    name: String,
    value: String,
}

/// Reference to an input module
#[derive(Debug, Clone, Default)]
struct IntermediateUseInputModule {
    name: String,
    template_vars: HashMap<String, String>,
}

// Resolved yard.yaml representation
//************************************************************************//

struct ResolvedYardFile {
    container_files: HashMap<String, Vec<ResolvedModule>>,
}

/// yard-module.yaml file
#[derive(Debug, Clone)]
struct ResolvedModule {
    containerfile: String,
    required_template_values: Vec<String>,
    optional_template_values: Vec<String>,
    /// source info for better errors
    source_info: SourceInfoKind,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
struct LocalModuleInfo {
    path: String,
    name: String,
}

impl SourceInfo for LocalModuleInfo {
    fn user_message(self) -> String {
        format!("Local path: {}", self.path)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
pub struct RemoteModuleInfo {
    pub url: String,
    pub commit: String,
    pub path: String,
    pub name: String,
}

impl SourceInfo for RemoteModuleInfo {
    fn user_message(self) -> String {
        format!(
            "Repo: {}\nCommit: {}\nRemote path: {}",
            self.url, self.commit, self.path
        )
    }
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
struct InlineModuleInfo {
    name: String,
}

impl SourceInfo for InlineModuleInfo {
    fn user_message(self) -> String {
        format!("Inline module: {}", self.name)
    }
}

#[enum_dispatch(SourceInfoKind)]
trait SourceInfo {
    fn user_message(self) -> String;
}

/// Info about where data came from.
#[enum_dispatch]
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum SourceInfoKind {
    LocalModuleInfo,
    RemoteModuleInfo,
    InlineModuleInfo,
}

//************************************************************************//

pub struct ModuleFiles {
    pub containerfile: PathBuf,
    pub module_file: PathBuf,
    pub source_info: SourceInfoKind,
}

/// parse yard.yaml and validate that all referenced modules are declared
fn parse_yard_yaml(path: &Path) -> anyhow::Result<IntermediateYardFile> {
    let yard_schema: &'static str = include_str!("./schemas/yard-schema.json");
    let yard_schema: serde_json::Value =
        serde_json::from_str(yard_schema).expect("yard-module-schema.json is not valid json");
    let compiled_schema = JSONSchema::options()
        .with_draft(Draft::Draft7)
        .compile(&yard_schema)
        .expect("yard-schema.json is not a valid json schema");

    let yard_yaml_file = File::open(path.join(YARD_YAML_FILE_NAME))
        .context(formatcp!("Could not open '{}'.", YARD_YAML_FILE_NAME))?;
    let yard_yaml: serde_yaml::Value = serde_yaml::from_reader(BufReader::new(yard_yaml_file))
        .context(formatcp!("{} is not valid yaml.", YARD_YAML_FILE_NAME))?;
    validate_against_schema(&compiled_schema, &yard_yaml, &path.display().to_string())?;
    let yard_yaml: YamlYard = serde_yaml::from_value(yard_yaml)?;

    let mut input_remotes: Vec<IntermediateRemote> = Vec::new();
    if let Some(remotes) = yard_yaml.inputs.remotes {
        for remote in remotes {
            input_remotes.push(IntermediateRemote {
                url: remote.url,
                commit: remote.commit,
                name_to_path: remote.paths,
            });
        }
    }
    let input_paths = yard_yaml.inputs.paths.unwrap_or_default();
    let output_container_files: HashMap<String, Vec<IntermediateUseModule>> = HashMap::new();
    for (containerfile_name, output) in yard_yaml.outputs {
        let mut modules: Vec<IntermediateUseModule> = Vec::new();
        for (module_name, module) in output {
            let Some(module) = module else {
                modules.push(IntermediateUseModule::Input(IntermediateUseInputModule {
                    name: module_name,
                    template_vars: HashMap::new(),
                }));
                continue;
            };
            match module {
                YamlOutput::String(value) => {
                    modules.push(IntermediateUseModule::Inline(IntermediateUseInlineModule {
                        name: module_name,
                        value,
                    }));
                }
                YamlOutput::StringMap(template_vars) => {
                    modules.push(IntermediateUseModule::Input(IntermediateUseInputModule {
                        name: module_name,
                        template_vars,
                    }));
                }
            }
        }
    }
    Ok(IntermediateYardFile {
        input_remotes,
        input_paths,
        output_container_files,
    })
}

/// resolve and validate fields in the yard.yaml file
async fn resolve_yard_yaml(yard_yaml: IntermediateYardFile) -> anyhow::Result<ResolvedYardFile> {
    let IntermediateYardFile {
        input_remotes,
        input_paths,
        output_container_files,
    } = yard_yaml;
    let mut module_to_files: HashMap<String, ModuleFiles> = HashMap::new();
    let mut module_names_are_unique_check: HashSet<String> = HashSet::new();
    for (name, path) in input_paths {
        if module_names_are_unique_check.contains(&name) {
            bail!(UserMessageError::new(format!(
                "A module with name '{}' is declared twice.",
                name
            )));
        }
        module_names_are_unique_check.insert(name.clone());
        module_to_files.insert(
            name.clone(),
            ModuleFiles {
                containerfile: PathBuf::from(&path).join(CONTAINERFILE_NAME),
                module_file: PathBuf::from(&path).join(MODULE_YAML_FILE_NAME),
                source_info: SourceInfoKind::LocalModuleInfo(LocalModuleInfo { path, name }),
            },
        );
    }
    for (name, path) in input_remotes.iter().flat_map(|e| e.name_to_path.iter()) {
        if module_names_are_unique_check.contains(&*name) {
            anyhow::bail!(UserMessageError::new(format!(
                "A module named '{}' is declared more than once",
                name
            )))
        }
    }
    download_remotes(input_remotes, &mut module_to_files).await?;
    let modules = resolve_modules(module_to_files)?;
    let mut containerfiles_to_parts: HashMap<String, Vec<ResolvedModule>> = HashMap::new();
    for (container_file_name, module_declarations) in output_container_files {
        let mut modules_for_container_file: Vec<ResolvedModule> = Vec::new();
        for module_declaration in module_declarations {
            match module_declaration {
                IntermediateUseModule::Inline(inline) => {
                    modules_for_container_file.push(ResolvedModule {
                        containerfile: inline.value,
                        required_template_values: Vec::with_capacity(0),
                        optional_template_values: Vec::with_capacity(0),
                        source_info: SourceInfoKind::InlineModuleInfo(InlineModuleInfo {
                            name: inline.name,
                        }),
                    });
                }
                IntermediateUseModule::Input(declared_module) => {
                    let module = modules.get(&declared_module.name).ok_or_else(|| {
                        UserMessageError::new(format!(
                            "Module '{}' is not declared as an input in the yard.yaml file.",
                            declared_module.name
                        ))
                    })?;
                    // validate
                    for required_template_arg in module.required_template_values.iter() {
                        if !declared_module
                            .template_vars
                            .contains_key(required_template_arg)
                        {
                            bail!(UserMessageError::new(format!(
                                "Template variable '{}' is required for module '{}'.",
                                required_template_arg, declared_module.name
                            )))
                        }
                    }
                    for template_var in declared_module.template_vars.keys() {
                        if !module.required_template_values.contains(template_var)
                            && !module.optional_template_values.contains(template_var)
                        {
                            bail!(UserMessageError::new(format!(
                                "Template variable '{}' is not defined in the module '{}'.",
                                template_var, declared_module.name
                            )))
                        }
                    }
                    modules_for_container_file.push(module.clone());
                }
            }
        }
        containerfiles_to_parts.insert(container_file_name, modules_for_container_file);
    }
    Ok(ResolvedYardFile {
        container_files: containerfiles_to_parts,
    })
}

async fn download_remotes(
    remotes: Vec<IntermediateRemote>,
    all_module_to_files: &mut HashMap<String, ModuleFiles>,
) -> anyhow::Result<()> {
    for remote in remotes {
        let git_provider = git_provider_from_url(&remote.url)?;
        let module_to_files = git_provider.get_module_files(&remote).await?;
        all_module_to_files.extend(module_to_files);
    }
    Ok(())
}

fn download_remote(url: &str, commit: &str) -> PathBuf {
    unimplemented!()
}

/// validates and builds all the referenced modules from the resolved paths.
fn resolve_modules(
    name_to_modulefiles: HashMap<String, ModuleFiles>,
) -> anyhow::Result<HashMap<String, ResolvedModule>> {
    let yard_module_schema: &'static str = include_str!("./schemas/yard-module-schema.json");
    let yard_module_schema: serde_json::Value = serde_json::from_str(yard_module_schema)
        .expect("yard-module-schema.json is not valid json");
    let compiled_schema = JSONSchema::options()
        .with_draft(Draft::Draft7)
        .compile(&yard_module_schema)
        .expect("yard-module-schema.json is not a valid json schema");
    let validate_schema_fn = |yaml: &serde_yaml::Value, source_name_or_path: &str| {
        validate_against_schema(&compiled_schema, yaml, source_name_or_path)
    };

    let mut modules: HashMap<String, ResolvedModule> = HashMap::new();
    for (name, module_files) in name_to_modulefiles {
        let module = create_module(module_files, validate_schema_fn)?;
        modules.insert(name, module);
    }
    return Ok(modules);
}

/// Validates and creates the internal module representation based off the path.
fn create_module<F: Fn(&serde_yaml::Value, &str) -> anyhow::Result<()>>(
    module_files: ModuleFiles,
    validate_schema_fn: F,
) -> anyhow::Result<ResolvedModule> {
    let module_yaml_path = module_files.module_file;
    if !module_yaml_path.is_file() {
        bail!(UserMessageError::new(
            formatcp!("{} does not exist.", MODULE_YAML_FILE_NAME).to_string()
        ))
    }
    let module_yaml_file = File::open(module_yaml_path.clone())
        .context(format!("Could not open '{}'.", module_yaml_path.display()))?;
    let yard_module_yaml: serde_yaml::Value =
        serde_yaml::from_reader(BufReader::new(module_yaml_file))
            .context("yard-module-schema.json is not valid json.")?;

    validate_schema_fn(&yard_module_yaml, &module_yaml_path.display().to_string())?;

    let raw_module: YamlModule = serde_yaml::from_value(yard_module_yaml)?;
    let args = raw_module.args.unwrap_or_default();
    let required_template_values = args.required.unwrap_or(Vec::with_capacity(0));
    let optional_template_values = args.optional.unwrap_or(Vec::with_capacity(0));

    let containerfile_path = module_files.containerfile;
    if !containerfile_path.is_file() {
        bail!(UserMessageError::new(
            "'Containerfile' does not exist in".to_string()
        ))
    }
    let containerfile = fs::read_to_string(containerfile_path)?;
    Ok(ResolvedModule {
        containerfile,
        required_template_values,
        optional_template_values,
        source_info: module_files.source_info,
    })
}

//************************************************************************//

fn validate_against_schema(
    compiled_schema: &JSONSchema,
    yaml: &serde_yaml::Value,
    source_name_or_path: &str,
) -> anyhow::Result<()> {
    let yaml_as_json = serde_json::to_value(&yaml).context(format!(
        "Could not convert the '{}' file to json for validation against the schema.",
        source_name_or_path
    ))?;
    compiled_schema
        .validate(&yaml_as_json)
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
        .context(UserMessageError::new(
            format!("{} does not follow the proper schema.", source_name_or_path).to_string(),
        ))?;
    Ok(())
}

//************************************************************************//

/// Apply args to each template and collect
fn apply_templates(yard: ResolvedYardFile) -> anyhow::Result<String> {
    unimplemented!()
}
