use std::{
    collections::{HashMap, HashSet},
    fmt::Debug,
    fs::{self, File},
    io::BufReader,
    path::{Path, PathBuf},
};

use anyhow::{bail, Context};
use const_format::formatcp;
use jsonschema::{Draft, JSONSchema};
use serde::{Deserialize, Serialize};
use tera::Tera;

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
    let containerfile = apply_templates(resolved_yard_file)?; // todo this should apply multiple
    fs::write("Containerfile", containerfile)?; // todo allow this path to be configured cli
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

//************************************************************************//

/// Builder for when constructing all the values needed to operate on the template
#[derive(Debug, Clone)]
struct ContainerfileTemplatePartBuilder {
    containerfile: String,
    required_template_values: HashSet<String>,
    optional_template_values: HashSet<String>,
    provided_template_values: HashMap<String, String>,
    /// source info for better errors
    source_info: SourceInfoKind,
}

impl ContainerfileTemplatePartBuilder {
    fn build(self) -> anyhow::Result<ContainerfileTemplatePart> {
        for var in self.required_template_values.iter() {
            if !self.provided_template_values.contains_key(var) {
                bail!(UserMessageError::new(format!(
                    "Required variable '{}' not found for:\n{}",
                    var,
                    self.source_info.source_location()
                )));
            }
        }
        for (var, val) in self.provided_template_values.iter() {
            if !self.required_template_values.contains(var)
                && !self.optional_template_values.contains(var)
            {
                bail!(UserMessageError::new(format!(
                    "Provided template variable '{}' not found in the module for:\n{}",
                    var,
                    self.source_info.source_location()
                )));
            }
        }
        Ok(ContainerfileTemplatePart {
            containerfile: self.containerfile,
            provided_template_values: self.provided_template_values,
            source_info: self.source_info,
        })
    }
}

// Resolved yard.yaml representation
//************************************************************************//

/// All containerfile templates. Ready to apply
struct ContainerfilesTemplates {
    /// Containerfile names to included modules
    container_files: HashMap<String, Vec<ContainerfileTemplatePart>>,
}

/// Containerfile file and yard-module.yaml file combined
#[derive(Debug, Clone)]
struct ContainerfileTemplatePart {
    containerfile: String,
    provided_template_values: HashMap<String, String>,
    /// source info for better errors
    source_info: SourceInfoKind,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
pub struct LocalModuleInfo {
    pub path: String,
    pub name: String,
}

impl SourceInfo for LocalModuleInfo {
    fn source_location(self) -> String {
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
    fn source_location(self) -> String {
        format!(
            "Repo: {}\nCommit: {}\nRemote path: {}",
            self.url, self.commit, self.path
        )
    }
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
pub struct InlineModuleInfo {
    pub name: String,
}

impl SourceInfo for InlineModuleInfo {
    fn source_location(self) -> String {
        format!("Inline module: {}", self.name)
    }
}

trait SourceInfo {
    fn source_location(self) -> String;
}

/// Info about where data came from.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum SourceInfoKind {
    LocalModuleInfo(LocalModuleInfo),
    RemoteModuleInfo(RemoteModuleInfo),
    InlineModuleInfo(InlineModuleInfo),
}

impl SourceInfo for SourceInfoKind {
    fn source_location(self) -> String {
        match self {
            SourceInfoKind::LocalModuleInfo(info) => info.source_location(),
            SourceInfoKind::RemoteModuleInfo(info) => info.source_location(),
            SourceInfoKind::InlineModuleInfo(info) => info.source_location(),
        }
    }
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

    let yard_yaml_file = File::open(path.join(YARD_YAML_FILE_NAME)).context(
        UserMessageError::new(formatcp!("Could not open '{}'.", YARD_YAML_FILE_NAME).to_string()),
    )?;
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
async fn resolve_yard_yaml(
    yard_yaml: IntermediateYardFile,
) -> anyhow::Result<ContainerfilesTemplates> {
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
    let mut containerfiles_to_parts: HashMap<String, Vec<ContainerfileTemplatePart>> =
        HashMap::new();
    for (container_file_name, module_declarations) in output_container_files {
        let mut modules_for_container_file: Vec<ContainerfileTemplatePart> = Vec::new();
        for module_declaration in module_declarations {
            match module_declaration {
                IntermediateUseModule::Inline(inline) => {
                    modules_for_container_file.push(
                        ContainerfileTemplatePartBuilder {
                            containerfile: inline.value,
                            required_template_values: HashSet::new(),
                            optional_template_values: HashSet::new(),
                            provided_template_values: HashMap::new(),
                            source_info: SourceInfoKind::InlineModuleInfo(InlineModuleInfo {
                                name: inline.name,
                            }),
                        }
                        .build()?,
                    );
                }
                IntermediateUseModule::Input(declared_module) => {
                    let module = modules.get(&declared_module.name).ok_or_else(|| {
                        UserMessageError::new(format!(
                            "Module '{}' is not declared as an input in the {} file.",
                            declared_module.name, YARD_YAML_FILE_NAME
                        ))
                    })?;
                    let mut module = module.clone();
                    for (var, val) in declared_module.template_vars {
                        module.provided_template_values.insert(var, val);
                    }
                    modules_for_container_file.push(module.build()?);
                }
            }
        }
        containerfiles_to_parts.insert(container_file_name, modules_for_container_file);
    }
    Ok(ContainerfilesTemplates {
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
) -> anyhow::Result<HashMap<String, ContainerfileTemplatePartBuilder>> {
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

    let mut modules: HashMap<String, ContainerfileTemplatePartBuilder> = HashMap::new();
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
) -> anyhow::Result<ContainerfileTemplatePartBuilder> {
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
    let required_template_values = args.required.unwrap_or(Vec::new()).into_iter().collect();
    let optional_template_values = args.optional.unwrap_or(Vec::new()).into_iter().collect();

    let containerfile_path = module_files.containerfile;
    if !containerfile_path.is_file() {
        bail!(UserMessageError::new(
            "'Containerfile' does not exist in".to_string()
        ))
    }
    let containerfile = fs::read_to_string(containerfile_path)?;
    Ok(ContainerfileTemplatePartBuilder {
        containerfile,
        required_template_values,
        optional_template_values,
        provided_template_values: HashMap::new(),
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
fn apply_templates(yard: ContainerfilesTemplates) -> anyhow::Result<String> {
    let mut tera = Tera::default();
    // No escaping, shouldn't matter though since we don't use these file types, but just to future proof.
    tera.autoescape_on(vec![]);
    tera.set_escape_fn(|e| e.to_string());

    let mut container_file_resolved_parts = Vec::new();
    for (containerfile_name, included_modules) in yard.container_files {
        for included_module in included_modules {
            let mut context = tera::Context::new();
            for (var, val) in included_module.provided_template_values {
                context.insert(var, &val);
            }
            let rendered_part = tera.render_str(&included_module.containerfile, &context);
            let rendered_part = match rendered_part {
                Ok(val) => val,
                Err(e) => Err(e).context(format!(
                    "Could not render template for Containerfile part found at:\n{}",
                    included_module.source_info.source_location(),
                ))?,
            };
            container_file_resolved_parts.push(rendered_part);
        }
    }
    Ok(container_file_resolved_parts.join("\n"))
}
