use std::{
    collections::HashMap,
    fmt::{Debug, Display},
    fs::{self, File},
    io::BufReader,
    path::{Path, PathBuf},
};

use anyhow::{bail, Context};
use const_format::formatcp;
use jsonschema::{Draft, JSONSchema};
use serde::{Deserialize, Serialize};

use crate::common::UserMessageError;

pub fn build(path: &Path) -> anyhow::Result<()> {
    let parsed_yard_file = parse_yard_yaml(path)?;
    let resolved_yard_file = resolve_yard_yaml(parsed_yard_file)?;
    let containerfile = apply_templates(resolved_yard_file);
    // todo write template file
    Ok(())
}

//************************************************************************//

#[derive(Debug, Clone, Default)]
struct RawYardFile {
    input_remotes: Vec<RawRemote>,
    /// Module name to path on local
    input_paths: HashMap<String, String>,
    /// Containerfile name to included modules
    output_container_files: HashMap<String, Vec<RawUseModule>>,
}

/// Reference to an input module or inline
#[derive(Debug, Clone)]
enum RawUseModule {
    Inline(RawUseInlineModule),
    Input(RawUseInputModule),
}

/// Inline module
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
struct RawUseInlineModule {
    name: String,
    value: String,
}

/// Reference to an input module
#[derive(Debug, Clone, Default)]
struct RawUseInputModule {
    name: String,
    template_vars: HashMap<String, String>,
}

/// Reference to a remote and containing modules
#[derive(Debug, Clone, Default)]
struct RawRemote {
    url: String,
    commit: String,
    name_to_path: HashMap<String, String>,
}

//************************************************************************//

struct ResolvedYardFile {
    container_files: HashMap<String, Vec<Module>>,
}

/// yard-module.yaml file
#[derive(Debug, Clone)]
struct Module {
    containerfile: String,
    required_template_values: Vec<String>,
    optional_template_values: Vec<String>,
    /// source info for better errors
    source_info: SourceInfo,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
struct LocalModuleInfo {
    path: String,
    name: String,
}

impl LocalModuleInfo {
    fn user_message(self) -> String {
        format!("Local path: {}", self.path)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
struct RemoteModuleInfo {
    repo_url: String,
    commit: String,
    path: String,
    name: String,
}

impl RemoteModuleInfo {
    fn user_message(self) -> String {
        format!(
            "Repo: {}\nCommit: {}\nRemote path: {}",
            self.repo_url, self.commit, self.path
        )
    }
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
struct InlineModuleInfo {
    name: String,
}

impl InlineModuleInfo {
    fn user_message(self) -> String {
        format!("Inline module: {}", self.name)
    }
}

/// Info about where data came from.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
enum SourceInfo {
    LocalModule(LocalModuleInfo),
    RemoteModule(RemoteModuleInfo),
    InlineModule(InlineModuleInfo),
}

impl SourceInfo {
    fn user_message(self) -> String {
        match self {
            SourceInfo::LocalModule(v) => v.user_message(),
            SourceInfo::RemoteModule(v) => v.user_message(),
            SourceInfo::InlineModule(v) => v.user_message(),
        }
    }
}

//************************************************************************//

const YARD_YAML_FILE_NAME: &str = "yard.yaml";

/// parse yard.yaml and validate that all referenced modules are declared
fn parse_yard_yaml(path: &Path) -> anyhow::Result<RawYardFile> {
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
    let yard_yaml: YardJsonSchema = serde_yaml::from_value(yard_yaml)?;

    let mut input_remotes: Vec<RawRemote> = Vec::new();
    if let Some(remotes) = yard_yaml.inputs.remotes {
        for remote in remotes {
            input_remotes.push(RawRemote {
                url: remote.url,
                commit: remote.commit,
                name_to_path: remote.paths,
            });
        }
    }
    let input_paths = yard_yaml.inputs.paths.unwrap_or_default();
    let output_container_files: HashMap<String, Vec<RawUseModule>> = HashMap::new();
    for (containerfile_name, output) in yard_yaml.outputs {
        let mut modules: Vec<RawUseModule> = Vec::new();
        for (module_name, module) in output {
            let Some(module) = module else {
                modules.push(RawUseModule::Input(RawUseInputModule {
                    name: module_name,
                    template_vars: HashMap::new(),
                }));
                continue;
            };
            match module {
                Output::String(value) => {
                    modules.push(RawUseModule::Inline(RawUseInlineModule {
                        name: module_name,
                        value,
                    }));
                }
                Output::StringMap(template_vars) => {
                    modules.push(RawUseModule::Input(RawUseInputModule {
                        name: module_name,
                        template_vars,
                    }));
                }
            }
        }
    }
    Ok(RawYardFile {
        input_remotes,
        input_paths,
        output_container_files,
    })
}

/// resolve and validate fields in the yard.yaml file
fn resolve_yard_yaml(yard_yaml: RawYardFile) -> anyhow::Result<ResolvedYardFile> {
    let RawYardFile {
        input_remotes,
        input_paths,
        output_container_files,
    } = yard_yaml;
    let mut input_module_name_to_path: HashMap<String, ResolvedPath> = HashMap::new();
    for (name, path) in input_paths {
        if input_module_name_to_path.contains_key(&name) {
            bail!(UserMessageError::new(format!(
                "A module with name '{}' is declared twice.",
                name
            )));
        }
        input_module_name_to_path.insert(
            name.clone(),
            ResolvedPath {
                path: path.clone().into(),
                source_info: SourceInfo::LocalModule(LocalModuleInfo { path, name }),
            },
        );
    }
    download_remotes(input_remotes, &mut input_module_name_to_path);
    let modules = resolve_modules(input_module_name_to_path)?;
    let mut containerfiles_to_parts: HashMap<String, Vec<Module>> = HashMap::new();
    for (container_file_name, module_declarations) in output_container_files {
        let mut modules_for_container_file: Vec<Module> = Vec::new();
        for module_declaration in module_declarations {
            match module_declaration {
                RawUseModule::Inline(inline) => {
                    modules_for_container_file.push(Module {
                        containerfile: inline.value,
                        required_template_values: Vec::with_capacity(0),
                        optional_template_values: Vec::with_capacity(0),
                        source_info: SourceInfo::InlineModule(InlineModuleInfo {
                            name: inline.name,
                        }),
                    });
                }
                RawUseModule::Input(declared_module) => {
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

struct ResolvedPath {
    path: PathBuf,
    source_info: SourceInfo,
}

fn download_remotes(remotes: Vec<RawRemote>, module_to_path: &mut HashMap<String, ResolvedPath>) {
    // todo make sure to return in a module with that name is already declared
    unimplemented!()
}

fn download_remote(url: &str, commit: &str) -> PathBuf {
    unimplemented!()
}

/// Apply args to each template and collect
fn apply_templates(yard: ResolvedYardFile) -> anyhow::Result<String> {
    unimplemented!()
}

/// validates and builds all the referenced modules from the resolved paths.
fn resolve_modules(
    module_to_path: HashMap<String, ResolvedPath>,
) -> anyhow::Result<HashMap<String, Module>> {
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

    let mut modules: HashMap<String, Module> = HashMap::new();
    for (name, resolved_path) in module_to_path {
        let module = create_module(
            resolved_path.path,
            validate_schema_fn,
            resolved_path.source_info,
        )?;
        modules.insert(name, module);
    }
    return Ok(modules);
}

const MODULE_YAML_FILE_NAME: &str = "yard-module.yaml";

/// Validates and creates the internal module representation based off the path.
fn create_module<F: Fn(&serde_yaml::Value, &str) -> anyhow::Result<()>>(
    module_dir: PathBuf,
    validate_schema_fn: F,
    source_info: SourceInfo,
) -> anyhow::Result<Module> {
    let module_yaml_path = module_dir.join(MODULE_YAML_FILE_NAME);
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

    validate_schema_fn(&yard_module_yaml, &module_dir.display().to_string())?;

    let raw_module: ModuleJsonSchema = serde_yaml::from_value(yard_module_yaml)?;
    let args = raw_module.args.unwrap_or_default();
    let required_template_values = args.required.unwrap_or(Vec::with_capacity(0));
    let optional_template_values = args.optional.unwrap_or(Vec::with_capacity(0));

    let containerfile_path = module_dir.join("Containerfile");
    if !containerfile_path.is_file() {
        bail!(UserMessageError::new(
            "'Containerfile' does not exist in".to_string()
        ))
    }
    let containerfile = fs::read_to_string(containerfile_path)?;
    Ok(Module {
        containerfile,
        required_template_values,
        optional_template_values,
        source_info,
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

/// Created using the yard-module-schema.json file and https://app.quicktype.io/
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct ModuleJsonSchema {
    pub args: Option<Args>,
    /// This is a modules description
    pub description: String,
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct Args {
    pub optional: Option<Vec<String>>,
    pub required: Option<Vec<String>>,
}

//************************************************************************//

/// Created using the yard-schema.json file and https://app.quicktype.io/
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct YardJsonSchema {
    pub inputs: Inputs,
    pub outputs: HashMap<String, HashMap<String, Option<Output>>>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Inputs {
    pub paths: Option<HashMap<String, String>>,
    pub remotes: Option<Vec<Remote>>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Remote {
    pub commit: String,
    pub paths: HashMap<String, String>,
    pub url: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum Output {
    String(String),
    StringMap(HashMap<String, String>),
}
