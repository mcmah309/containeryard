use core::str;
use std::{
    collections::{HashMap, HashSet},
    fmt::Debug,
    path::{Component, Path, PathBuf},
};

use eros::Context;
use indexmap::IndexMap;
use jsonschema::{Draft, Validator};
use serde::{Deserialize, Serialize};
use tera::Tera;
use tokio::fs;
use tracing::trace;

use crate::{
    remote_resolvers::{GitProvider, create_provider},
    user_error::user_error,
};

pub const YARD_YAML_FILE_NAME: &str = "yard.yaml";

pub async fn build(
    path: &Path,
    do_not_refetch: bool,
    with_cache_busting: bool,
    ignore_requires: Vec<String>,
    ignore_all_requires: bool,
) -> eros::Result<()> {
    let (parsed_yard_file, post_build_hook) = parse_yard_yaml(path).await?;
    let ignore_requires = ignore_requires.into_iter().collect();
    let resolved_yard_file = resolve_yard_yaml(
        parsed_yard_file,
        path,
        do_not_refetch,
        &ignore_requires,
        ignore_all_requires,
    )
    .await?;
    if resolved_yard_file.name_to_module.is_empty() {
        return Err(user_error(
            "No modules were resolved. Add at least one module to an output in yard.yaml.",
        ));
    }
    let outputs = apply_templating(resolved_yard_file, with_cache_busting)?;
    if outputs.is_empty() {
        return Err(user_error(
            "No Containerfiles were created. Add at least one entry under `outputs` in yard.yaml.",
        ));
    }
    for (file_name, content) in outputs {
        let file_path = path.join(&file_name);
        fs::write(&file_path, content)
            .await
            .with_context(|| format!("Could not write to '{}'.", file_path.display()))
            .with_user_context(|| {
                format!(
                    "Could not write output '{}'. Check that the destination is writable.",
                    file_name
                )
            })?;
        println!(
            "Created '{}' at '{}'",
            file_name,
            file_path.canonicalize().unwrap_or(file_path).display()
        );
    }

    if let Some(post_build_hook) = post_build_hook {
        duct_sh::sh_dangerous(&post_build_hook)
            .run()
            .with_context(|| format!("Post-build hook `{post_build_hook}` failed"))
            .user_context("The post-build hook failed. Review the hook and try it manually.")?;
    }
    Ok(())
}

// Deserialized module config
//************************************************************************//
/// Created using the yard-module-schema.json file and https://app.quicktype.io/
#[derive(Debug, Clone, PartialEq, Default, Deserialize)]
pub struct YamlModule {
    pub args: Option<YamlArgs>,
    /// This is a modules description
    pub description: Option<String>,
    /// If true, this module is split into build, install, and optional finalize fragments. The
    /// build fragment is hoisted to the start of the generated Containerfile, the install fragment
    /// is injected where the module is declared, and the finalize fragment is appended after all
    /// declared modules. Defaults to false.
    #[serde(default)]
    pub split: bool,
    /// Module files that must be included earlier in each output that uses this module. Paths are
    /// relative to this module file.
    pub requires: Option<Vec<String>>,
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

/// Check by creating using the yard-schema.json file and https://app.quicktype.io/ __has been modified__
#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct YamlYard {
    pub hooks: Option<YamlHooks>,
    pub inputs: YamlInputs,
    /// Containerfile name to config
    pub outputs: IndexMap<String, Vec<YamlModuleType>>,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct YamlHooks {
    pub build: YamlBuildHooks,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct YamlBuildHooks {
    pub pre: Option<String>,
    pub post: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct YamlInputs {
    pub modules: Option<HashMap<String, String>>,
    pub remotes: Option<Vec<YamlRemote>>,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct YamlRemote {
    pub commit: String,
    pub modules: HashMap<String, String>,
    pub url: String,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(untagged)]
pub enum YamlModuleType {
    /// Inline `- Run ...`
    Inline(String),
    /// Module ref `- module_name:`
    /// Module ref with template values `- module_name: ...`
    InputRef(IndexMap<String, Option<HashMap<String, TemplateValue>>>),
}

/// Argument values retain their type when inserted into template contexts.
#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
#[serde(untagged)]
pub enum TemplateValue {
    String(String),
    Boolean(bool),
    Number(serde_json::Number),
}

// Intermediate  yard.yaml reprsentation
//************************************************************************//

#[derive(Debug, Clone, Default)]
struct YardFile {
    input_remotes: Vec<RemoteModules>,
    /// Module name to path on local
    input_modules: HashMap<String, String>,
    /// Containerfile name to included modules
    output_container_files: IndexMap<String, Vec<UseModule>>,
}

/// Reference to a remote and containing modules
#[derive(Debug, Clone, Default)]
pub struct RemoteModules {
    pub url: String,
    pub commit: String,
    pub name_to_path: HashMap<String, String>,
}

/// Reference to an input module or inline
#[derive(Debug, Clone)]
enum UseModule {
    Inline(UseInlineModule),
    Input(UseInputModule),
    Output(String),
}

/// Inline module
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
struct UseInlineModule {
    value: String,
}

/// Reference to an input module
#[derive(Debug, Clone, Default)]
struct UseInputModule {
    name: String,
    template_vars: HashMap<String, TemplateValue>,
}

//************************************************************************//

/// Builder for when constructing all the values needed to operate on the template
#[derive(Debug, Clone)]
struct ModuleBuilder {
    containerfile_data: String,
    /// Install fragment template for split modules. `None` for regular modules.
    install_fragment_data: Option<String>,
    /// Optional fragment appended after all declared modules.
    finalize_fragment_data: Option<String>,
    /// Whether this module is split across output positions.
    split: bool,
    required_modules: Vec<ModuleRequirement>,
    required_files: Vec<String>,
    required_template_values: HashSet<String>,
    optional_template_values: HashSet<String>,
    provided_template_values: HashMap<String, TemplateValue>,
    /// source info for better errors
    source_info: SourceInfoKind,
    /// Module name for cache-busting aliases (None if not applicable)
    name: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
enum ModuleOrigin {
    Local,
    Remote { url: String, commit: String },
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct ModuleIdentity {
    origin: ModuleOrigin,
    path: PathBuf,
}

#[derive(Debug, Clone)]
struct ModuleRequirement {
    declared_path: String,
    resolved: ModuleIdentity,
}

impl ModuleBuilder {
    fn build(self) -> eros::Result<Module> {
        for var in self.required_template_values.iter() {
            if !self.provided_template_values.contains_key(var) {
                return Err(user_error(format!(
                    "Required variable '{}' was not provided for {}.",
                    var,
                    self.source_info.user_label()
                ))
                .context(format!(
                    "Missing required variable for {}",
                    self.source_info.source_location()
                )));
            }
        }
        for (var, val) in self.provided_template_values.iter() {
            if !self.required_template_values.contains(var)
                && !self.optional_template_values.contains(var)
            {
                return Err(user_error(format!(
                    "Template variable '{}' is not accepted by {}.",
                    var,
                    self.source_info.user_label()
                ))
                .context(format!(
                    "Unexpected template variable for {}",
                    self.source_info.source_location()
                )));
            }
        }
        // This is not necessary at this point, as this should have already been checked. But kept just to make sure.
        validate_path_references(&self.required_files)?;
        if self.split && self.install_fragment_data.is_none() {
            return Err(user_error(format!(
                "{} is marked as split (`split: true`) but has no install fragment. Split modules require at least two Containerfile blocks: a build fragment followed by an install fragment.",
                self.source_info.user_label()
            ))
            .context(self.source_info.source_location()));
        }
        if !self.split && self.install_fragment_data.is_some() {
            return Err(user_error(format!(
                "{} has multiple Containerfile blocks but is not marked as split. Add `split: true` to its configuration or combine the blocks.",
                self.source_info.user_label()
            ))
            .context(self.source_info.source_location()));
        }
        Ok(Module {
            containerfile_template: self.containerfile_data,
            install_fragment_template: self.install_fragment_data,
            finalize_fragment_template: self.finalize_fragment_data,
            split: self.split,
            provided_template_values: self.provided_template_values,
            source_info: self.source_info,
            name: self.name,
        })
    }
}

// Resolved yard.yaml representation
//************************************************************************//

/// All containerfile and their resolved modules. Ready to apply
struct Containerfiles {
    /// Containerfile names to included modules
    name_to_module: IndexMap<String, Vec<Module>>,
}

/// The template Containerfile and config combined. Ready to apply
#[derive(Debug, Clone)]
struct Module {
    containerfile_template: String,
    /// Install fragment template for split modules. `None` for regular modules.
    install_fragment_template: Option<String>,
    /// Optional fragment appended after all declared modules.
    finalize_fragment_template: Option<String>,
    /// Whether this module is split across output positions.
    split: bool,
    provided_template_values: HashMap<String, TemplateValue>,
    /// source info for better errors
    source_info: SourceInfoKind,
    /// Module name used for cache-busting aliases (None if not applicable)
    name: Option<String>,
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
    pub repo_owner: String,
    pub repo_name: String,
    pub commit: String,
    pub path: String,
    /// Module name
    pub name: String,
}

impl SourceInfo for RemoteModuleInfo {
    fn source_location(&self) -> String {
        format!(
            "Remote url: '{}', owner: '{}', repo: '{}', commit: '{}', path: '{}', name: '{}'",
            self.url, self.repo_owner, self.repo_name, self.commit, self.path, self.name
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
    Local(LocalModuleInfo),
    Remote(RemoteModuleInfo),
    Inline(InlineModuleInfo),
}

impl SourceInfoKind {
    fn user_label(&self) -> String {
        match self {
            SourceInfoKind::Local(info) => format!("local module '{}'", info.name),
            SourceInfoKind::Remote(info) => format!("remote module '{}'", info.name),
            SourceInfoKind::Inline(_) => "an inline module".to_owned(),
        }
    }

    fn label(&self) -> String {
        match self {
            SourceInfoKind::Local(info) => format!("{}: {}", info.name, info.path),
            SourceInfoKind::Remote(info) => format!("{}: {}", info.name, info.path),
            SourceInfoKind::Inline(_) => "~INLINE~".to_owned(),
        }
    }

    fn module_identity(&self) -> eros::Result<Option<ModuleIdentity>> {
        let (origin, path) = match self {
            SourceInfoKind::Local(info) => (ModuleOrigin::Local, info.path.as_str()),
            SourceInfoKind::Remote(info) => (
                ModuleOrigin::Remote {
                    url: info.url.clone(),
                    commit: info.commit.clone(),
                },
                info.path.as_str(),
            ),
            SourceInfoKind::Inline(_) => return Ok(None),
        };
        Ok(Some(ModuleIdentity {
            origin,
            path: normalize_module_path(Path::new(path))?,
        }))
    }
}

impl SourceInfo for SourceInfoKind {
    fn source_location(&self) -> String {
        match self {
            SourceInfoKind::Local(info) => info.source_location(),
            SourceInfoKind::Remote(info) => info.source_location(),
            SourceInfoKind::Inline(info) => info.source_location(),
        }
    }
}

//************************************************************************//

pub struct ModuleFileData {
    pub containerfile_data: String,
    pub config_data: String,
    /// Install fragment for split modules. `None` when only one Containerfile block is present.
    pub install_fragment_data: Option<String>,
    /// Optional finalize fragment for split modules.
    pub finalize_fragment_data: Option<String>,
    pub source_info: SourceInfoKind,
}

async fn load_yard_file(
    compiled_schema: &Validator,
    yard_file_path: &Path,
) -> eros::Result<YamlYard> {
    let yard_yaml_file_data = fs::read_to_string(yard_file_path)
        .await
        .with_context(|| format!("Could not read '{}'.", yard_file_path.display()))
        .user_context("Could not read yard.yaml. Check that it exists and is readable.")?;
    let yard_yaml: serde_yaml::Value = serde_yaml::from_str(&yard_yaml_file_data)
        .with_context(|| format!("'{}' is not valid YAML.", yard_file_path.display()))
        .user_context("yard.yaml contains invalid YAML. Check its syntax and indentation.")?;
    validate_against_schema(compiled_schema, &yard_yaml)
        .with_context(|| format!("Validate schema for '{}'.", yard_file_path.display()))
        .user_context(
            "yard.yaml does not match the expected schema. Check its field names and value types.",
        )?;
    let yard_yaml: YamlYard = serde_yaml::from_value(yard_yaml).with_context(|| {
        format!(
            "Was able to serialize '{}', but was unable to convert to internal expected model.",
            yard_file_path.display()
        )
    })?;
    Ok(yard_yaml)
}

fn yard_validator() -> Validator {
    let yard_schema: &'static str = include_str!("./schemas/yard-schema.json");
    let yard_schema: serde_json::Value =
        serde_json::from_str(yard_schema).expect("yard-module-schema.json is not valid json");
    Validator::options()
        .with_draft(Draft::Draft7)
        .build(&yard_schema)
        .expect("yard-schema.json is not a valid json schema")
}

pub async fn output_order(path: &Path) -> eros::Result<Vec<String>> {
    let yard_file_path = path.join(YARD_YAML_FILE_NAME);
    let validator = yard_validator();
    let yard_yaml = load_yard_file(&validator, &yard_file_path).await?;
    Ok(yard_yaml.outputs.keys().cloned().collect())
}

/// parse yard.yaml and validate that all referenced modules are declared
#[eros::context("Could not parse '{}'.", YARD_YAML_FILE_NAME)]
async fn parse_yard_yaml(path: &Path) -> eros::Result<(YardFile, Option<String>)> {
    let validator = yard_validator();
    let yard_file_path = path.join(YARD_YAML_FILE_NAME);
    let mut yard_yaml = load_yard_file(&validator, &yard_file_path).await?;
    let pre_build_hook: Option<&str> = (|| yard_yaml.hooks.as_ref()?.build.pre.as_deref())();
    if let Some(pre_build_hook) = pre_build_hook {
        duct_sh::sh_dangerous(pre_build_hook)
            .run()
            .with_context(|| format!("Pre-build hook `{pre_build_hook}` failed"))
            .user_context("The pre-build hook failed. Review the hook and try it manually.")?;
        // We need to reload in case the pre-build hook updates the file
        yard_yaml = load_yard_file(&validator, &yard_file_path)
            .await
            .context("First load of yard file succeeded, second load failed")?;
    }

    let mut input_remotes: Vec<RemoteModules> = Vec::new();
    if let Some(remotes) = yard_yaml.inputs.remotes {
        for remote in remotes {
            input_remotes.push(RemoteModules {
                url: remote.url,
                commit: remote.commit,
                name_to_path: remote.modules,
            });
        }
    }
    let input_modules = yard_yaml.inputs.modules.unwrap_or_default();
    let input_names: HashSet<String> = input_modules
        .keys()
        .chain(
            input_remotes
                .iter()
                .flat_map(|remote| remote.name_to_path.keys()),
        )
        .cloned()
        .collect();
    let output_names: HashSet<String> = yard_yaml.outputs.keys().cloned().collect();
    let mut output_container_files: IndexMap<String, Vec<UseModule>> = IndexMap::new();
    for (containerfile_name, output) in yard_yaml.outputs {
        let mut modules: Vec<UseModule> = Vec::new();
        for module in output {
            match module {
                YamlModuleType::Inline(value) => {
                    modules.push(UseModule::Inline(UseInlineModule { value }));
                }
                YamlModuleType::InputRef(module_ref) => {
                    assert!(
                        module_ref.len() <= 1,
                        "Internal model is wrong. This should be `- module_name: ...`"
                    );
                    for (module_name, template_vars) in module_ref {
                        if output_names.contains(&module_name)
                            && !input_names.contains(&module_name)
                        {
                            if template_vars.is_some() {
                                return Err(user_error(format!(
                                    "Output reference '{module_name}' cannot have arguments. Declare it as `- {module_name}:`."
                                )));
                            }
                            modules.push(UseModule::Output(module_name));
                            continue;
                        }
                        modules.push(UseModule::Input(UseInputModule {
                            name: module_name,
                            template_vars: template_vars.unwrap_or_default(),
                        }));
                    }
                }
            };
        }
        output_container_files.insert(containerfile_name, modules);
    }
    let post_build_hook: Option<String> = (|| yard_yaml.hooks?.build.post)();
    Ok((
        YardFile {
            input_remotes,
            input_modules,
            output_container_files,
        },
        post_build_hook,
    ))
}

/// Expand output references into their module declarations before resolving modules. A reference
/// uses module-style syntax and has a name that exactly matches another output.
fn expand_output_references(
    outputs: IndexMap<String, Vec<UseModule>>,
) -> eros::Result<IndexMap<String, Vec<UseModule>>> {
    fn expand(
        output_name: &str,
        outputs: &IndexMap<String, Vec<UseModule>>,
        expanded: &mut HashMap<String, Vec<UseModule>>,
        visiting: &mut Vec<String>,
    ) -> eros::Result<Vec<UseModule>> {
        if let Some(modules) = expanded.get(output_name) {
            return Ok(modules.clone());
        }

        if let Some(cycle_start) = visiting.iter().position(|name| name == output_name) {
            let mut cycle = visiting[cycle_start..].to_vec();
            cycle.push(output_name.to_owned());
            return Err(user_error(format!(
                "Output reference cycle detected: {}. Remove one of the references in the cycle.",
                cycle
                    .iter()
                    .map(|name| format!("'{name}'"))
                    .collect::<Vec<_>>()
                    .join(" -> ")
            )));
        }

        let declarations = outputs
            .get(output_name)
            .expect("Output references are created only for known output names");
        visiting.push(output_name.to_owned());

        let mut modules = Vec::new();
        for declaration in declarations {
            match declaration {
                UseModule::Output(referenced_output) => {
                    modules.extend(expand(referenced_output, outputs, expanded, visiting)?)
                }
                declaration => modules.push(declaration.clone()),
            }
        }

        visiting.pop();
        expanded.insert(output_name.to_owned(), modules.clone());
        Ok(modules)
    }

    let mut cache = HashMap::new();
    let mut expanded_outputs = IndexMap::new();
    for output_name in outputs.keys() {
        let modules = expand(output_name, &outputs, &mut cache, &mut Vec::new())?;
        expanded_outputs.insert(output_name.clone(), modules);
    }
    Ok(expanded_outputs)
}

/// resolve and validate fields in the yard.yaml file
#[eros::context(
    "Could not resolve all the fields in the parsed '{}' file",
    YARD_YAML_FILE_NAME
)]
async fn resolve_yard_yaml(
    yard_yaml: YardFile,
    path: &Path,
    do_not_refetch: bool,
    ignore_requires: &HashSet<String>,
    ignore_all_requires: bool,
) -> eros::Result<Containerfiles> {
    let YardFile {
        input_remotes,
        input_modules,
        output_container_files,
    } = yard_yaml;
    let output_container_files = expand_output_references(output_container_files)?;
    assert!(!output_container_files.is_empty(), "Ouputs should exist");
    let mut local_name_to_module_files_data: HashMap<String, ModuleFileData> = HashMap::new();
    let mut module_names_are_unique_check: HashSet<String> = HashSet::new();
    for (name, path) in input_modules {
        if module_names_are_unique_check.contains(&name) {
            return Err(user_error(format!(
                "Module '{name}' is declared more than once. Give every input module a unique name."
            )));
        }
        module_names_are_unique_check.insert(name.clone());
        let module_data = read_module_file(&PathBuf::from(&path))
            .await
            .with_context(|| format!("Load local module '{name}' from '{path}'"))
            .with_user_context(|| {
                format!(
                    "Could not load local module '{name}'. Check its path and file permissions."
                )
            })?;
        local_name_to_module_files_data.insert(
            name.clone(),
            ModuleFileData {
                containerfile_data: module_data.containerfile,
                config_data: module_data.config,
                install_fragment_data: module_data.install_fragment,
                finalize_fragment_data: module_data.finalize_fragment,
                source_info: SourceInfoKind::Local(LocalModuleInfo { path, name }),
            },
        );
    }
    for (name, path) in input_remotes.iter().flat_map(|e| e.name_to_path.iter()) {
        if module_names_are_unique_check.contains(name) {
            return Err(user_error(format!(
                "Module '{name}' is declared more than once. Give every input module a unique name."
            )));
        }
    }

    let remote_name_to_module_files: HashMap<String, ModuleFileData> =
        retrieve_module_file_data(input_remotes).await?;
    local_name_to_module_files_data.extend(remote_name_to_module_files);
    let name_to_module_files_data = local_name_to_module_files_data;
    let modules: HashMap<String, ModuleBuilder> =
        validate_schema_and_create_module_builders(name_to_module_files_data).await?;

    // Resolve
    resolve_additional_files(&modules, path, do_not_refetch).await?;
    let mut containerfiles_to_parts: IndexMap<String, Vec<Module>> = IndexMap::new();
    for (container_file_name, module_declarations) in output_container_files {
        let mut modules_for_container_file: Vec<Module> = Vec::new();
        let mut seen_module_names: HashSet<String> = HashSet::new();
        let mut seen_module_paths: HashSet<ModuleIdentity> = HashSet::new();
        let mut inline_counter = 0u32;
        for module_declaration in module_declarations {
            match module_declaration {
                UseModule::Inline(inline) => {
                    let synthetic_name = format!("inline_{inline_counter}");
                    inline_counter += 1;
                    modules_for_container_file.push(
                        ModuleBuilder {
                            containerfile_data: inline.value.clone(),
                            install_fragment_data: None,
                            finalize_fragment_data: None,
                            split: false,
                            required_modules: Vec::new(),
                            required_files: Vec::new(),
                            required_template_values: HashSet::new(),
                            optional_template_values: HashSet::new(),
                            provided_template_values: HashMap::new(),
                            source_info: SourceInfoKind::Inline(InlineModuleInfo {
                                value: inline.value,
                            }),
                            name: Some(synthetic_name),
                        }
                        .build()?,
                    );
                }
                UseModule::Input(declared_module) => {
                    if !seen_module_names.insert(declared_module.name.clone()) {
                        return Err(user_error(format!(
                            "Module '{}' is declared more than once in the output '{}'. Each input module may only be declared once per output.",
                            declared_module.name, container_file_name
                        )));
                    }
                    let module = modules.get(&declared_module.name).ok_or_else(|| {
                        user_error(format!(
                            "Module '{}' is used by an output but is not declared under `inputs` in {}.",
                            declared_module.name, YARD_YAML_FILE_NAME
                        ))
                    })?;
                    if !ignore_all_requires && !ignore_requires.contains(&declared_module.name) {
                        for requirement in &module.required_modules {
                            if !seen_module_paths.contains(&requirement.resolved) {
                                return Err(user_error(format!(
                                    "Module '{}' requires module '{}' to be included before it in output '{}'.",
                                    declared_module.name,
                                    requirement.declared_path,
                                    container_file_name
                                ))
                                .context(format!(
                                    "Required module resolved to '{}' for {}",
                                    requirement.resolved.path.display(),
                                    module.source_info.source_location()
                                )));
                            }
                        }
                    }
                    let mut module = module.clone();
                    module.name = Some(declared_module.name.clone());
                    for (var, val) in declared_module.template_vars {
                        let val = match val {
                            TemplateValue::String(value) => {
                                TemplateValue::String(resolve_template_value(value)?)
                            }
                            TemplateValue::Boolean(value) => TemplateValue::Boolean(value),
                            TemplateValue::Number(value) => TemplateValue::Number(value),
                        };
                        module.provided_template_values.insert(var, val);
                    }
                    let module_identity = module
                        .source_info
                        .module_identity()?
                        .expect("Input modules always have a module identity");
                    modules_for_container_file.push(module.build()?);
                    seen_module_paths.insert(module_identity);
                }
                UseModule::Output(_) => {
                    unreachable!("Output references should have been expanded")
                }
            }
        }
        containerfiles_to_parts.insert(container_file_name, modules_for_container_file);
    }
    Ok(Containerfiles {
        name_to_module: containerfiles_to_parts,
    })
}

#[eros::context("Could not retrieve module file data")]
async fn retrieve_module_file_data(
    remotes: Vec<RemoteModules>,
) -> eros::Result<HashMap<String, ModuleFileData>> {
    let mut name_to_module_file_data: HashMap<String, ModuleFileData> = HashMap::new();
    for remote in remotes {
        let git_provider = create_provider(remote.url, remote.commit)
            .user_context("A remote URL in yard.yaml is invalid. Use an HTTP(S) or SSH Git URL.")?;
        trace!("Identified provider '{:?}'", git_provider);
        let name_to_module_files_data_part =
            git_provider
                .retrieve_module(remote.name_to_path)
                .await
                .user_context(
                    "Could not retrieve remote modules. Check the repository URL, commit, network connection, and Git credentials.",
                )?;
        name_to_module_file_data.extend(name_to_module_files_data_part);
    }
    Ok(name_to_module_file_data)
}

#[eros::context("Could not resolve additional required files")]
async fn resolve_additional_files(
    name_to_module: &HashMap<String, ModuleBuilder>,
    local_download_path_root: &Path,
    do_not_refetch: bool,
) -> eros::Result<()> {
    for (name, module) in name_to_module {
        match module.source_info {
            SourceInfoKind::Local(ref local) => {
                let local_file_path = local_download_path_root.join(&local.path);
                validate_path_references(&[local_file_path]).with_user_context(|| {
                    format!(
                        "A required file for local module '{name}' is missing or has an invalid path."
                    )
                })?;
            }
            SourceInfoKind::Remote(ref remote) => {
                let git_provider = create_provider(remote.url.clone(), remote.commit.clone())?;
                for file_path in module.required_files.iter() {
                    let local_download_path = local_download_path_root.join(file_path);
                    if local_download_path.exists() && do_not_refetch {
                        println!(
                            "Note: '{}' is not refetched since it already exists and `--do-not-refetch` is set.",
                            local_download_path.display()
                        );
                        continue;
                    }
                    let remote_file_path = format!(
                        "{}/{}",
                        PathBuf::from(&remote.path).parent().unwrap().display(),
                        file_path
                    );
                    git_provider
                        .retrieve_file_and_put_at(&remote_file_path, &local_download_path)
                        .await
                        .with_context(|| {
                            format!(
                                "Could not download '{}' at\n{}",
                                file_path,
                                remote.source_location()
                            )
                        })
                        .with_user_context(|| {
                            format!(
                                "Could not download required file '{file_path}' for module '{name}'. Check the remote path and repository access."
                            )
                        })?;
                }
            }
            SourceInfoKind::Inline(_) => {}
        }
    }
    Ok(())
}

fn validate_path_references<T: AsRef<Path>>(files: &[T]) -> eros::Result<()> {
    for file in files {
        let file = file.as_ref();
        let path = PathBuf::from(file);
        is_local_absolute(&path)?;
        if !path.exists() {
            return Err(user_error(
                "A required path does not exist. Check the module's `required_files` entries.",
            )
            .context(format!("Required path '{}' does not exist", file.display())));
        }
    }
    Ok(())
}

/// No "~" or ".."
fn is_local_absolute(path: &Path) -> eros::Result<()> {
    let error = || {
        user_error(
            "A configured path is invalid. Paths must be relative and cannot contain `~` or `..` components.",
        )
        .context(format!("Invalid configured path: '{}'", path.display()))
    };
    for component in path.components() {
        match component {
            Component::Prefix(_) => return Err(error()),
            Component::RootDir | Component::ParentDir => return Err(error()),
            Component::Normal(os_str) if os_str == "~" => return Err(error()),
            _ => (),
        }
    }
    Ok(())
}

/// Normalize a module path without touching the filesystem. Module paths are rooted at either the
/// yard directory or a remote repository, so they may not escape that root.
fn normalize_module_path(path: &Path) -> eros::Result<PathBuf> {
    let error = || {
        user_error(
            "A module path is invalid. Module paths must be relative and stay within their source root.",
        )
        .context(format!("Invalid module path: '{}'", path.display()))
    };
    let mut normalized = PathBuf::new();
    for component in path.components() {
        match component {
            Component::Prefix(_) | Component::RootDir => return Err(error()),
            Component::ParentDir => {
                if !normalized.pop() {
                    return Err(error());
                }
            }
            Component::CurDir => {}
            Component::Normal(part) => normalized.push(part),
        }
    }
    if normalized.as_os_str().is_empty() {
        return Err(error());
    }
    Ok(normalized)
}

fn resolve_module_requirement(
    source_info: &SourceInfoKind,
    required_path: String,
) -> eros::Result<ModuleRequirement> {
    let source = source_info
        .module_identity()?
        .expect("Only file-backed modules can declare requirements");
    let requirement_path = Path::new(&required_path);
    if requirement_path.is_absolute() {
        return Err(user_error(format!(
            "Required module path '{}' is not valid. Required module paths must be relative to the module file.",
            required_path
        )));
    }
    let resolved_path = normalize_module_path(
        &source
            .path
            .parent()
            .unwrap_or_else(|| Path::new(""))
            .join(requirement_path),
    )?;
    Ok(ModuleRequirement {
        declared_path: required_path,
        resolved: ModuleIdentity {
            origin: source.origin,
            path: resolved_path,
        },
    })
}

#[eros::context("Could not validate and construct the referenced modules")]
async fn validate_schema_and_create_module_builders(
    name_to_module_files_data: HashMap<String, ModuleFileData>,
) -> eros::Result<HashMap<String, ModuleBuilder>> {
    let yard_module_schema: &'static str = include_str!("./schemas/yard-module-schema.json");
    let yard_module_schema: serde_json::Value = serde_json::from_str(yard_module_schema)
        .expect("yard-module-schema.json is not valid json");
    let compiled_schema = Validator::options()
        .with_draft(Draft::Draft7)
        .build(&yard_module_schema)
        .expect("yard-module-schema.json is not a valid json schema");
    let validate_module_schema_fn =
        |yaml: &serde_yaml::Value| validate_against_schema(&compiled_schema, yaml);

    let mut modules: HashMap<String, ModuleBuilder> = HashMap::new();
    for (name, module_files) in name_to_module_files_data {
        let module =
            validate_and_create_module_builder(module_files, validate_module_schema_fn).await?;
        modules.insert(name, module);
    }

    for (index, (name1, module1)) in modules.iter().enumerate() {
        for (name1, module2) in modules.iter().skip(index + 1) {
            for required_file1 in &module1.required_files {
                for required_file2 in &module2.required_files {
                    if required_file1 == required_file2 {
                        return Err(user_error(format!(
                            "Required file '{}' is declared by both {} and {}. The modules would overwrite each other's file.",
                            required_file1,
                            module1.source_info.user_label(),
                            module2.source_info.user_label()
                        ))
                        .context(format!(
                            "Conflicting sources:\n{}\n{}",
                            module1.source_info.source_location(),
                            module2.source_info.source_location()
                        )));
                    }
                }
            }
        }
    }

    Ok(modules)
}

/// Validates and creates the internal module representation.
async fn validate_and_create_module_builder<F: Fn(&serde_yaml::Value) -> eros::Result<()>>(
    module_files: ModuleFileData,
    validate_module_schema_fn: F,
) -> eros::Result<ModuleBuilder> {
    let (
        required_modules,
        required_files,
        required_template_values,
        optional_template_values,
        split,
    ) = (|| -> eros::Result<_> {
        // If there is no config block, default to a regular, non-split module.
        let yard_module_yaml: serde_yaml::Value = if module_files.config_data.trim().is_empty() {
            serde_yaml::Value::Null
        } else {
            serde_yaml::from_str(&module_files.config_data)
                .context("Parse module configuration as YAML")
                .user_context(
                    "A module contains invalid YAML. Check the module's configuration block syntax and indentation.",
                )?
        };

        validate_module_schema_fn(&yard_module_yaml)
            .context("Module schema validation failed")
            .user_context(
                "A module configuration does not match the expected schema. Check its field names and value types.",
            )?;

        let raw_module: YamlModule = serde_yaml::from_value(yard_module_yaml).context(
            "Was able to serialize yaml, but was unable to convert to internal expected model.",
        )?;
        fn tera_accepts_ident(name: &str) -> bool {
            let template = format!("{{{{ {} }}}}", name);
            let mut context = tera::Context::new();
            context.insert(name.to_owned(), "");
            tera::Tera::one_off(&template, &context, false).is_ok_and(|e| e.is_empty())
        }
        let YamlModule {
            args,
            split,
            requires,
            required_files,
            ..
        } = raw_module;
        let args = args.unwrap_or_default();
        let required_files = required_files.unwrap_or_default();
        let required_modules = requires
            .unwrap_or_default()
            .into_iter()
            .map(|path| resolve_module_requirement(&module_files.source_info, path))
            .collect::<eros::Result<Vec<_>>>()?;
        let required_template_values: HashSet<String> =
            args.required.unwrap_or_default().into_iter().collect();
        let optional_template_values: HashSet<String> =
            args.optional.unwrap_or_default().into_iter().collect();
        for template_value in required_template_values
            .iter()
            .chain(optional_template_values.iter())
        {
            if !tera_accepts_ident(template_value) {
                return Err(user_error(format!(
                    "Template variable '{}' is not a valid identifier for a module argument.",
                    template_value
                )));
            }
        }

        for required_file in required_files.iter() {
            is_local_absolute(&PathBuf::from(required_file))?;
        }
        Ok((
            required_modules,
            required_files,
            required_template_values,
            optional_template_values,
            split,
        ))
    })()
    .with_context(|| module_files.source_info.source_location())
    .with_user_context(|| {
        format!(
            "Could not use {}. Check its configuration and contents.",
            module_files.source_info.user_label()
        )
    })?;

    Ok(ModuleBuilder {
        containerfile_data: module_files.containerfile_data,
        install_fragment_data: module_files.install_fragment_data,
        finalize_fragment_data: module_files.finalize_fragment_data,
        split,
        required_modules,
        required_files,
        required_template_values,
        optional_template_values,
        provided_template_values: HashMap::new(),
        source_info: module_files.source_info,
        name: None,
    })
}

//************************************************************************//

fn validate_against_schema(
    compiled_schema: &Validator,
    yaml: &serde_yaml::Value,
) -> eros::Result<()> {
    let yaml_as_json = serde_json::to_value(yaml)
        .context("Could not convert to json for validation against the schema.")?;
    compiled_schema
        .validate(&yaml_as_json)
        .map_err(|error| {
            eros::error!(
                r#"Validation error: 

                Issue: {}

                Violation Instance: {}

                Violation Path: {}

                Schema Property Violated: {}"#,
                &error.to_string(),
                &error.instance(),
                &error.instance_path(),
                &error.schema_path()
            )
        })
        .context("YAML does not follow the expected schema")?;
    Ok(())
}

//************************************************************************//

fn resolve_template_value(val: String) -> eros::Result<String> {
    // shell command
    if val.starts_with("$(") && val.ends_with(")") {
        let command = &val[2..val.len() - 1];
        let output = duct_sh::sh_dangerous(command).read().map_err(|e| {
            eros::error!(
                "Failed to execute command '{}' for template value: {}",
                command,
                e
            )
        })
        .user_context(
            "A command used as a template value failed. Review the command in yard.yaml and try it manually.",
        )?;
        return Ok(output.trim().to_string());
    }
    // env var
    if let Some(var) = val.strip_prefix("$") {
        let val = std::env::var(var)
            .with_context(|| format!("Could not get environment variable '{var}'"))
            .with_user_context(|| {
                format!(
                    "Environment variable '{var}' is required by yard.yaml but is not available."
                )
            })?;
        return Ok(val);
    }
    Ok(val)
}

//************************************************************************//

/// Contianfile name and file text
type Outputs = Vec<(String, String)>;

/// Apply args to each template and collect
fn apply_templating(yard: Containerfiles, with_cache_busting: bool) -> eros::Result<Outputs> {
    let mut tera = Tera::default();
    // No escaping, shouldn't matter though since we don't use these file types, but just to future proof.
    tera.autoescape_on(Vec::<&str>::new());
    tera.set_escape_fn(|e, writer| writer.write(e.as_bytes()).map(|_| ()));

    /// Renders a single template with the provided values, attaching source info on error.
    fn render(
        tera: &Tera,
        template: &str,
        provided_template_values: &HashMap<String, TemplateValue>,
        source_info: &SourceInfoKind,
    ) -> eros::Result<String> {
        let mut context = tera::Context::new();
        for (var, val) in provided_template_values {
            context.insert(var.clone(), val);
        }
        let rendered = tera.render_str(template, &context, false);
        let rendered = match rendered {
            Ok(val) => val,
            Err(e) => Err(e).with_context(|| {
                format!(
                    "Could not render template for Containerfile part found at:\n{}",
                    source_info.source_location(),
                )
            })
            .user_context(
                "Could not render a module template. Check its template syntax and supplied arguments.",
            )?,
        };
        Ok(rendered.trim().to_string())
    }

    let mut outputs = Vec::new();
    for (containerfile_name, included_modules) in yard.name_to_module {
        // Build fragments of split modules are hoisted to the start of the Containerfile, while
        // finalize fragments are appended after all declared modules.
        let mut build_fragment_parts: Vec<String> = Vec::new();
        let mut container_file_resolved_parts = Vec::new();
        let mut finalize_fragment_parts: Vec<String> = Vec::new();
        for included_module in included_modules {
            let label = included_module.source_info.label();
            if included_module.split {
                // Hoist the build fragment to the start.
                let mut build_fragment = render(
                    &tera,
                    &included_module.containerfile_template,
                    &included_module.provided_template_values,
                    &included_module.source_info,
                )?;
                if with_cache_busting {
                    let name = included_module
                        .name
                        .as_deref()
                        .expect("Should be provided at this point");
                    build_fragment = apply_cache_busting(&build_fragment, name);
                }
                let part = format!("####  {label} (build fragment)  ####\n\n{build_fragment}\n");
                build_fragment_parts.push(part);
                // Inject the install fragment where the module is declared.
                let install_template = included_module
                    .install_fragment_template
                    .as_ref()
                    .expect("Split modules must have an install fragment; this is checked in ModuleBuilder::build");
                let mut install_fragment = render(
                    &tera,
                    install_template,
                    &included_module.provided_template_values,
                    &included_module.source_info,
                )?;
                if with_cache_busting {
                    let name = included_module
                        .name
                        .as_deref()
                        .expect("Should be provided at this point");
                    install_fragment = apply_cache_busting(&install_fragment, name);
                }
                let part =
                    format!("####  {label} (install fragment)  ####\n\n{install_fragment}\n");
                container_file_resolved_parts.push(part);

                if let Some(finalize_template) = included_module.finalize_fragment_template.as_ref()
                {
                    let mut finalize_fragment = render(
                        &tera,
                        finalize_template,
                        &included_module.provided_template_values,
                        &included_module.source_info,
                    )?;
                    if with_cache_busting {
                        let name = included_module
                            .name
                            .as_deref()
                            .expect("Should be provided at this point");
                        finalize_fragment = apply_cache_busting(&finalize_fragment, name);
                    }
                    let part =
                        format!("####  {label} (finalize fragment)  ####\n\n{finalize_fragment}\n");
                    finalize_fragment_parts.push(part);
                }
            } else {
                let mut rendered_part = render(
                    &tera,
                    &included_module.containerfile_template,
                    &included_module.provided_template_values,
                    &included_module.source_info,
                )?;
                if with_cache_busting {
                    let module_name = included_module
                        .name
                        .as_deref()
                        .expect("Should be provided at this point");
                    rendered_part = apply_cache_busting(&rendered_part, module_name);
                }
                let part = format!("####  {label}  ####\n\n{rendered_part}\n");
                container_file_resolved_parts.push(part);
            }
        }
        let mut all_parts = build_fragment_parts;
        all_parts.extend(container_file_resolved_parts);
        all_parts.extend(finalize_fragment_parts);
        outputs.push((containerfile_name, all_parts.join("\n")));
    }
    Ok(outputs)
}

//************************************************************************//

#[derive(PartialEq)]
enum CapturingState {
    None,
    Containerfile,
    Config,
}

pub struct ModuleData {
    pub containerfile: String,
    pub config: String,
    /// Install fragment for split modules. `None` when only one Containerfile block is present.
    pub install_fragment: Option<String>,
    /// Optional third fragment appended after all declared modules.
    pub finalize_fragment: Option<String>,
}

#[eros::context("Could not read '{}' as a module.", &PathBuf::from(&path).display())]
pub async fn read_module_file(path: &Path) -> eros::Result<ModuleData> {
    let data = fs::read_to_string(path)
        .await
        .with_context(|| format!("Read module file '{}'", path.display()))
        .user_context(
            "Could not read a referenced module file. Check that it exists and is readable.",
        )?;
    // Collect Containerfile blocks in the order they appear. Split modules use a build fragment,
    // an install fragment, and optionally a finalize fragment.
    let mut container_data: Vec<String> = Vec::new();
    let mut config_data = None;
    let mut capture_status = CapturingState::None;
    let mut capture = String::new();
    for line in data.lines() {
        let compare_line = line.trim().to_lowercase();
        if compare_line == "```yaml" {
            if config_data.is_some() {
                continue;
            }
            if capture_status != CapturingState::None {
                return Err(user_error(
                    "A module starts a YAML block before closing the previous fenced block.",
                ));
            }
            capture_status = CapturingState::Config;
            continue;
        } else if compare_line == "```containerfile" || compare_line == "```dockerfile" {
            if capture_status != CapturingState::None {
                return Err(user_error(
                    "A module starts a Containerfile block before closing the previous fenced block.",
                ));
            }
            capture_status = CapturingState::Containerfile;
            continue;
        } else if compare_line == "```" {
            match capture_status {
                CapturingState::None => {
                    // Could be another documentation block ignore
                }
                CapturingState::Containerfile => {
                    container_data.push(capture.clone());
                    capture.clear();
                    capture_status = CapturingState::None;
                }
                CapturingState::Config => {
                    config_data = Some(capture.clone());
                    capture.clear();
                    capture_status = CapturingState::None;
                }
            }
            continue;
        }
        if capture_status != CapturingState::None {
            capture.push_str(line);
            capture.push('\n');
        }
    }
    match capture_status {
        CapturingState::Containerfile => {
            return Err(user_error(
                "A module has an unclosed Containerfile fenced block. Add a closing ``` line.",
            ));
        }
        CapturingState::Config => {
            return Err(user_error(
                "A module has an unclosed YAML fenced block. Add a closing ``` line.",
            ));
        }
        CapturingState::None => {}
    }
    match (container_data.is_empty(), config_data) {
        (true, None) => {
            // No sections found for either so interpret the entire file as a containerfile
            Ok(ModuleData {
                containerfile: data,
                config: String::new(),
                install_fragment: None,
                finalize_fragment: None,
            })
        }
        (true, Some(_)) => Err(user_error(
            "A module has a YAML configuration block but no Containerfile block.",
        )),
        (false, config_data) => {
            if container_data.len() > 3 {
                return Err(user_error(format!(
                    "A module has {} Containerfile blocks, but at most three are supported. Regular modules use one block; split modules use a build fragment, an install fragment, and an optional finalize fragment.",
                    container_data.len()
                )));
            }
            let mut fragments = container_data.into_iter();
            Ok(ModuleData {
                containerfile: fragments.next().expect("container_data is not empty"),
                config: config_data.unwrap_or_default(),
                install_fragment: fragments.next(),
                finalize_fragment: fragments.next(),
            })
        }
    }
}

fn apply_cache_busting(containerfile: &str, module_name: &str) -> String {
    let module_name = module_name.replace("-", "_").to_uppercase();
    format!("ARG CACHE_BUST_{module_name}=1\n{containerfile}")
}
