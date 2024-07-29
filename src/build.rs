use core::str;
use std::{
    collections::{HashMap, HashSet},
    fmt::Debug,
    path::{Component, Path, PathBuf},
    process::Command,
};

use anyhow::{anyhow, bail, Context};
use const_format::formatcp;
use jsonschema::{Draft, JSONSchema};
use serde::{Deserialize, Serialize};
use tera::Tera;
use tokio::fs;

use crate::{
    common::UserMessageError,
    git::{git_provider_from_url, GitProvider},
};

pub const MODULE_YAML_FILE_NAME: &str = "yard-module.yaml";
pub const YARD_YAML_FILE_NAME: &str = "yard.yaml";
pub const CONTAINERFILE_NAME: &str = "Containerfile";

pub async fn build(path: &Path) -> anyhow::Result<()> {
    let parsed_yard_file = parse_yard_yaml(path).await.with_context(|| {
        UserMessageError::new(formatcp!("Could not parse '{}'.", YARD_YAML_FILE_NAME).to_string())
    })?;
    let resolved_yard_file = resolve_yard_yaml(parsed_yard_file, path)
        .await
        .with_context(|| {
            UserMessageError::new(
                formatcp!(
                    "Could not resolve all the fields in the parsed '{}' file",
                    YARD_YAML_FILE_NAME
                )
                .to_string(),
            )
        })?;
    if resolved_yard_file.name_to_module.is_empty() {
        bail!(UserMessageError::new(
            "No modules were resolved.".to_string()
        ))
    }
    let outputs = apply_templates(resolved_yard_file)
        .with_context(|| UserMessageError::new("Could not apply templates".to_string()))?;
    if outputs.is_empty() {
        bail!(UserMessageError::new(
            "No Containerfiles where created.".to_string()
        ))
    }
    for (file_name, content) in outputs {
        let file_path = path.join(&file_name);
        fs::write(&file_path, content).await.with_context(|| {
            UserMessageError::new(format!("Could not write to '{}'.", &file_name).to_string())
        })?;
        println!(
            "Created '{}' at '{}",
            &file_name,
            &file_path
                .canonicalize()
                .expect("Could not get absolute path.")
                .display()
        );
    }
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
    /// List of required files for the module. Must be absolution paths from the current directory without a starting "/"
    pub required_files: Option<Vec<String>>,
}

#[derive(Debug, Clone, PartialEq, Default, Deserialize)]
pub struct YamlArgs {
    pub optional: Option<Vec<String>>,
    pub required: Option<Vec<String>>,
}

// Deserialized yard.yaml
//************************************************************************//

/// Created by using the yard-schema.json file and https://app.quicktype.io/ __has been modified__
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct YamlYard {
    pub inputs: YamlInputs,
    /// Containerfile name to config
    pub outputs: HashMap<String, Vec<YamlModuleType>>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct YamlInputs {
    pub modules: Option<HashMap<String, String>>,
    pub remotes: Option<Vec<YamlRemote>>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct YamlRemote {
    pub commit: String,
    pub modules: HashMap<String, String>,
    pub url: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum YamlModuleType {
    /// Inline `- Run ...`
    Inline(String),
    /// Module ref `- module_name:`
    /// Module ref with template values `- module_name: ...`
    InputRef(HashMap<String, Option<HashMap<String, String>>>),
}

// Intermediate  yard.yaml reprsentation
//************************************************************************//

#[derive(Debug, Clone, Default)]
struct IntermediateYardFile {
    input_remotes: Vec<IntermediateRemote>,
    /// Module name to path on local
    input_modules: HashMap<String, String>,
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
struct ModuleBuilder {
    containerfile_data: String,
    required_files: Vec<String>,
    required_template_values: HashSet<String>,
    optional_template_values: HashSet<String>,
    provided_template_values: HashMap<String, String>,
    /// source info for better errors
    source_info: SourceInfoKind,
}

impl ModuleBuilder {
    fn build(self) -> anyhow::Result<Module> {
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
        // This is not necessary at this point, as this should have already been checked. But kept just to make sure.
        validate_path_references(&self.required_files)?;
        Ok(Module {
            containerfile_template: self.containerfile_data,
            provided_template_values: self.provided_template_values,
            source_info: self.source_info,
        })
    }
}

// Resolved yard.yaml representation
//************************************************************************//

/// All containerfile and their resolved modules. Ready to apply
struct Containerfiles {
    /// Containerfile names to included modules
    name_to_module: HashMap<String, Vec<Module>>,
}

/// The template Containerfile file and yard-module.yaml file combined. Ready to apply
#[derive(Debug, Clone)]
struct Module {
    containerfile_template: String,
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
    fn source_location(&self) -> String {
        format!("Local path: {}", self.path)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
pub struct RemoteModuleInfo {
    /// original url
    pub url: String,
    pub owner: String,
    pub repo: String,
    pub commit: String,
    pub path: String,
    /// Module name
    pub name: String,
}

impl SourceInfo for RemoteModuleInfo {
    fn source_location(&self) -> String {
        format!(
            "Remote url: '{}', owner: '{}', repo: '{}', commit: '{}', path: '{}', name: '{}'",
            self.url, self.owner, self.repo, self.commit, self.path, self.name
        )
    }
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
pub struct InlineModuleInfo {
    pub value: String,
}

impl SourceInfo for InlineModuleInfo {
    fn source_location(&self) -> String {
        format!("Inline module value: {}", self.value)
    }
}

trait SourceInfo {
    fn source_location(&self) -> String;
}

/// Info about where data came from.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum SourceInfoKind {
    LocalModuleInfo(LocalModuleInfo),
    RemoteModuleInfo(RemoteModuleInfo),
    InlineModuleInfo(InlineModuleInfo),
}

impl SourceInfo for SourceInfoKind {
    fn source_location(&self) -> String {
        match self {
            SourceInfoKind::LocalModuleInfo(info) => info.source_location(),
            SourceInfoKind::RemoteModuleInfo(info) => info.source_location(),
            SourceInfoKind::InlineModuleInfo(info) => info.source_location(),
        }
    }
}

//************************************************************************//

pub struct ModuleFilesData {
    pub containerfile_data: String,
    pub module_file_data: String,
    pub source_info: SourceInfoKind,
}

/// parse yard.yaml and validate that all referenced modules are declared
async fn parse_yard_yaml(path: &Path) -> anyhow::Result<IntermediateYardFile> {
    let yard_schema: &'static str = include_str!("./schemas/yard-schema.json");
    let yard_schema: serde_json::Value =
        serde_json::from_str(yard_schema).expect("yard-module-schema.json is not valid json");
    let compiled_schema = JSONSchema::options()
        .with_draft(Draft::Draft7)
        .compile(&yard_schema)
        .expect("yard-schema.json is not a valid json schema");

    let yard_file_path = path.join(YARD_YAML_FILE_NAME);
    let yard_yaml_file_data = fs::read_to_string(&yard_file_path).await.with_context(|| {
        UserMessageError::new(format!("Could read '{}'.", &yard_file_path.display()).to_string())
    })?;
    let yard_yaml: serde_yaml::Value = serde_yaml::from_str(&yard_yaml_file_data)
        .with_context(|| format!("{} is not valid yaml.", &yard_file_path.display()))?;
    validate_against_schema(&compiled_schema, &yard_yaml)
        .with_context(|| format!("For path '{}'.", &yard_file_path.display()))?;
    let yard_yaml: YamlYard = serde_yaml::from_value(yard_yaml).context(UserMessageError::new(
        format!(
            "Was able to serialize '{}', but was unable to convert to internal expected model.",
            yard_file_path.display()
        )
        .to_string(),
    ))?;

    let mut input_remotes: Vec<IntermediateRemote> = Vec::new();
    if let Some(remotes) = yard_yaml.inputs.remotes {
        for remote in remotes {
            input_remotes.push(IntermediateRemote {
                url: remote.url,
                commit: remote.commit,
                name_to_path: remote.modules,
            });
        }
    }
    let input_modules = yard_yaml.inputs.modules.unwrap_or_default();
    let mut output_container_files: HashMap<String, Vec<IntermediateUseModule>> = HashMap::new();
    for (containerfile_name, output) in yard_yaml.outputs {
        let mut modules: Vec<IntermediateUseModule> = Vec::new();
        for module in output {
            match module {
                YamlModuleType::Inline(value) => {
                    modules.push(IntermediateUseModule::Inline(IntermediateUseInlineModule {
                        value,
                    }));
                }
                YamlModuleType::InputRef(module_ref) => {
                    assert!(
                        module_ref.len() <= 1,
                        "Internal model is wrong. This should be `- module_name: ...`"
                    );
                    for (module_name, template_vars) in module_ref {
                        modules.push(IntermediateUseModule::Input(IntermediateUseInputModule {
                            name: module_name,
                            template_vars: template_vars.unwrap_or_default(),
                        }));
                    }
                }
            };
        }
        output_container_files.insert(containerfile_name, modules);
    }
    Ok(IntermediateYardFile {
        input_remotes,
        input_modules,
        output_container_files,
    })
}

/// resolve and validate fields in the yard.yaml file
async fn resolve_yard_yaml(
    yard_yaml: IntermediateYardFile,
    path: &Path,
) -> anyhow::Result<Containerfiles> {
    let IntermediateYardFile {
        input_remotes,
        input_modules,
        output_container_files,
    } = yard_yaml;
    assert!(!output_container_files.is_empty(), "Ouputs should exist");
    let mut local_name_to_module_files_data: HashMap<String, ModuleFilesData> = HashMap::new();
    let mut module_names_are_unique_check: HashSet<String> = HashSet::new();
    for (name, path) in input_modules {
        if module_names_are_unique_check.contains(&name) {
            bail!(UserMessageError::new(format!(
                "A module with name '{}' is declared twice.",
                name
            )));
        }
        module_names_are_unique_check.insert(name.clone());
        let containerfile_file = PathBuf::from(&path).join(CONTAINERFILE_NAME);
        let module_file = PathBuf::from(&path).join(MODULE_YAML_FILE_NAME);
        let containerfile_data: String = fs::read_to_string(&containerfile_file)
            .await
            .context(format!(
                "Could not read '{}' to string.",
                &containerfile_file.display()
            ))?
            .into();
        let module_file_data: String = fs::read_to_string(&module_file)
            .await
            .context(format!(
                "Could not read '{}' to string.",
                &module_file.display()
            ))?
            .into();
        local_name_to_module_files_data.insert(
            name.clone(),
            ModuleFilesData {
                containerfile_data,
                module_file_data,
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

    let remote_name_to_module_files: HashMap<String, ModuleFilesData> =
        download_remotes(input_remotes).await.with_context(|| {
            UserMessageError::new("Failed to download some remotes.".to_string())
        })?;
    local_name_to_module_files_data.extend(remote_name_to_module_files);
    let name_to_module_files_data = local_name_to_module_files_data;
    let modules: HashMap<String, ModuleBuilder> =
        validate_schema_and_create_module_builders(name_to_module_files_data)
            .await
            .context("Could not resolve modules.")?;

    // Resolve
    resolve_additional_files(&modules, path)
        .await
        .context("Could not resolve additional required files")?;
    let mut containerfiles_to_parts: HashMap<String, Vec<Module>> = HashMap::new();
    for (container_file_name, module_declarations) in output_container_files {
        let mut modules_for_container_file: Vec<Module> = Vec::new();
        for module_declaration in module_declarations {
            match module_declaration {
                IntermediateUseModule::Inline(inline) => {
                    modules_for_container_file.push(
                        ModuleBuilder {
                            containerfile_data: inline.value.clone(),
                            required_files: Vec::new(),
                            required_template_values: HashSet::new(),
                            optional_template_values: HashSet::new(),
                            provided_template_values: HashMap::new(),
                            source_info: SourceInfoKind::InlineModuleInfo(InlineModuleInfo {
                                value: inline.value,
                            }),
                        }
                        .build()?,
                    );
                }
                IntermediateUseModule::Input(declared_module) => {
                    let module = modules.get(&declared_module.name).ok_or_else(|| {
                        UserMessageError::new(format!(
                            "Module '{}' is not declared as an input in the '{}' file.",
                            declared_module.name, YARD_YAML_FILE_NAME
                        ))
                    })?;
                    let mut module = module.clone();
                    for (var, val) in declared_module.template_vars {
                        let val = resolve_template_value(val)?;
                        module.provided_template_values.insert(var, val);
                    }
                    modules_for_container_file.push(module.build()?);
                }
            }
        }
        containerfiles_to_parts.insert(container_file_name, modules_for_container_file);
    }
    Ok(Containerfiles {
        name_to_module: containerfiles_to_parts,
    })
}

async fn download_remotes(
    remotes: Vec<IntermediateRemote>,
) -> anyhow::Result<HashMap<String, ModuleFilesData>> {
    let mut name_to_module_file_data: HashMap<String, ModuleFilesData> = HashMap::new();
    for remote in remotes {
        let git_provider = git_provider_from_url(&remote.url)?;
        let name_to_module_files_data_part = git_provider.get_module_files(&remote).await?;
        name_to_module_file_data.extend(name_to_module_files_data_part);
    }
    Ok(name_to_module_file_data)
}

async fn resolve_additional_files(
    name_to_module: &HashMap<String, ModuleBuilder>,
    local_download_path_root: &Path,
) -> anyhow::Result<()> {
    for (name, module) in name_to_module {
        match module.source_info {
            SourceInfoKind::LocalModuleInfo(ref local) => {
                let local_file_path = local_download_path_root.join(&local.path);
                validate_path_references(&[local_file_path])?;
            }
            SourceInfoKind::RemoteModuleInfo(ref remote) => {
                let git_provider = git_provider_from_url(&remote.url)?;
                for file_path in module.required_files.iter() {
                    let local_download_path = local_download_path_root.join(&file_path);
                    if local_download_path.exists() {
                        println!(
                            "Found '{}' locally. Not downloading.",
                            &local_download_path.display()
                        );
                        continue;
                    }
                    is_local_absolute(&local_download_path)?;
                    let remote_file_path = format!("{}/{}", remote.path, file_path);
                    git_provider
                        .download_file(
                            &remote.owner,
                            &remote.repo,
                            &remote.commit,
                            &remote_file_path,
                            &local_download_path,
                        )
                        .await
                        .with_context(|| {
                            UserMessageError::new(format!(
                                "Could not download '{}' at\n{}",
                                &file_path,
                                remote.source_location()
                            ))
                        })?;
                }
            }
            SourceInfoKind::InlineModuleInfo(_) => {}
        }
    }
    Ok(())
}

fn validate_path_references<T: AsRef<Path>>(files: &[T]) -> anyhow::Result<()> {
    for file in files {
        let file = file.as_ref();
        let path = PathBuf::from(file);
        is_local_absolute(&path)?;
        if !path.exists() {
            bail!(UserMessageError::new(format!(
                "Path '{}' does not exist, but it should at this point.",
                file.display()
            )));
        }
    }
    Ok(())
}

/// No "~" or ".."
fn is_local_absolute(path: &Path) -> anyhow::Result<()> {
    let error = || {
        UserMessageError::new(format!(
            "Path '{}' is not valid. Paths must be relative containing no '~' or '..' components.",
            path.display()
        ))
    };
    for component in path.components() {
        match component {
            Component::Prefix(_) => bail!(error()),
            Component::RootDir | Component::ParentDir => bail!(error()),
            Component::Normal(os_str) if os_str == "~" => bail!(error()),
            _ => (),
        }
    }
    Ok(())
}

async fn validate_schema_and_create_module_builders(
    name_to_module_files_data: HashMap<String, ModuleFilesData>,
) -> anyhow::Result<HashMap<String, ModuleBuilder>> {
    let yard_module_schema: &'static str = include_str!("./schemas/yard-module-schema.json");
    let yard_module_schema: serde_json::Value = serde_json::from_str(yard_module_schema)
        .expect("yard-module-schema.json is not valid json");
    let compiled_schema = JSONSchema::options()
        .with_draft(Draft::Draft7)
        .compile(&yard_module_schema)
        .expect("yard-module-schema.json is not a valid json schema");
    let validate_module_schema_fn =
        |yaml: &serde_yaml::Value| validate_against_schema(&compiled_schema, yaml);

    let mut modules: HashMap<String, ModuleBuilder> = HashMap::new();
    for (name, module_files) in name_to_module_files_data {
        let module = validate_and_create_module_builder(module_files, validate_module_schema_fn)
            .await
            .with_context(|| {
                UserMessageError::new("Failed to validate and create module builder.".to_string())
            })?;
        modules.insert(name, module);
    }
    return Ok(modules);
}

/// Validates and creates the internal module representation.
async fn validate_and_create_module_builder<F: Fn(&serde_yaml::Value) -> anyhow::Result<()>>(
    module_files: ModuleFilesData,
    validate_module_schema_fn: F,
) -> anyhow::Result<ModuleBuilder> {
    let (required_files, required_template_values, optional_template_values) =
        (|| -> anyhow::Result<_> {
            let yard_module_yaml: serde_yaml::Value =
                serde_yaml::from_str(&module_files.module_file_data)
                    .with_context(|| "yard-module-schema.json is not valid json.")?;

            validate_module_schema_fn(&yard_module_yaml).context("Schema validation failed.")?;

            let raw_module: YamlModule = serde_yaml::from_value(yard_module_yaml).context(
                "Was able to serialize yaml, but was unable to convert to internal expected model.",
            )?;
            let args = raw_module.args.unwrap_or_default();
            let required_files = raw_module.required_files.unwrap_or_default();
            let required_template_values = args.required.unwrap_or_default().into_iter().collect();
            let optional_template_values = args.optional.unwrap_or_default().into_iter().collect();

            Ok((
                required_files,
                required_template_values,
                optional_template_values,
            ))
        })()
        .context(UserMessageError::new(
            module_files.source_info.source_location(),
        ))?;

    Ok(ModuleBuilder {
        containerfile_data: module_files.containerfile_data,
        required_files: required_files,
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
) -> anyhow::Result<()> {
    let yaml_as_json = serde_json::to_value(&yaml)
        .context("Could not convert to json for validation against the schema.")?;
    compiled_schema
        .validate(&yaml_as_json)
        .map_err(|errors| {
            let mut error_message = String::new();
            for error in errors {
                error_message.push_str(&format!(
                    "Validation error: '{}'\n\tInstance path: '{}'\n\tSchema path: '{}'",
                    error, error.instance_path, error.schema_path
                ));
            }
            UserMessageError::new(error_message)
        })
        .with_context(|| {
            UserMessageError::new("yaml does not follow the proper schema.".to_string())
        })?;
    Ok(())
}

//************************************************************************//

fn resolve_template_value(val: String) -> anyhow::Result<String> {
    // shell command
    if val.starts_with("$(") && val.ends_with(")") {
        let command = &val[2..val.len() - 1];
        let output = Command::new("sh")
            .arg("-c")
            .arg(command)
            .output()
            .map_err(|e| {
                anyhow!(
                    "Failed to execute command '{}' for template value: {}",
                    command,
                    e
                )
            })?;
        if !output.status.success() {
            bail!(
                "Command '{}' failed with status: {}",
                command,
                output.status
            );
        }
        let val = str::from_utf8(&output.stdout)?;
        return Ok(val.trim().to_string());
    }
    // env var
    if val.starts_with("$") {
        let var = &val[1..];
        let val = std::env::var(var)
            .with_context(|| format!("Could not get env var '{}' for template value.", var))?;
        return Ok(val);
    }
    Ok(val)
}

//************************************************************************//

/// Contianfile name and file text
type Outputs = Vec<(String, String)>;

/// Apply args to each template and collect
fn apply_templates(yard: Containerfiles) -> anyhow::Result<Outputs> {
    let mut tera = Tera::default();
    // No escaping, shouldn't matter though since we don't use these file types, but just to future proof.
    tera.autoescape_on(vec![]);
    tera.set_escape_fn(|e| e.to_string());

    let mut outputs = Vec::new();
    let mut container_file_resolved_parts = Vec::new();
    for (containerfile_name, included_modules) in yard.name_to_module {
        for included_module in included_modules {
            let mut context = tera::Context::new();
            for (var, val) in included_module.provided_template_values {
                context.insert(var, &val);
            }
            let rendered_part = tera.render_str(&included_module.containerfile_template, &context);
            let rendered_part = match rendered_part {
                Ok(val) => val,
                Err(e) => Err(e).with_context(|| {
                    UserMessageError::new(format!(
                        "Could not render template for Containerfile part found at:\n{}",
                        included_module.source_info.source_location(),
                    ))
                })?,
            };
            container_file_resolved_parts.push(rendered_part);
        }
        outputs.push((containerfile_name, container_file_resolved_parts.join("\n")));
        container_file_resolved_parts.clear();
    }
    Ok(outputs)
}
