use std::collections::HashMap;

use crate::{build::ModuleData, scripts::Script, user_error::user_error};

enum BlockKind {
    Config,
    Containerfile(String),
    Script { name: String, interpreter: String },
    Documentation,
}

#[derive(Debug, Clone, Default)]
pub struct Fragments {
    pub build: Option<String>,
    pub main: Option<String>,
    pub finalize: Option<String>,
}

impl Fragments {
    fn is_empty(&self) -> bool {
        self.build.is_none() && self.main.is_none() && self.finalize.is_none()
    }

    fn insert(&mut self, name: &str, body: String) -> eros::Result<()> {
        let slot = match name {
            "build" => &mut self.build,
            "main" => &mut self.main,
            "finalize" => &mut self.finalize,
            _ => unreachable!("Fragment names are validated when opening a fence"),
        };
        if slot.replace(body).is_some() {
            return Err(user_error(format!(
                "Containerfile fragment '{name}' is declared more than once in a module. Each fragment name may appear once."
            )));
        }
        Ok(())
    }
}

struct Fence {
    marker: char,
    length: usize,
    indent: usize,
    kind: BlockKind,
    body: String,
}

fn opening_fence(line: &str) -> eros::Result<Option<Fence>> {
    let trimmed = line.trim_start_matches(' ');
    let indent = line.len() - trimmed.len();
    let Some(marker @ ('`' | '~')) = trimmed.chars().next() else {
        return Ok(None);
    };
    let length = trimmed.chars().take_while(|c| *c == marker).count();
    if length < 3 {
        return Ok(None);
    }
    let mut info = trimmed[length..].split_whitespace();
    let language = info.next().unwrap_or_default().to_ascii_lowercase();
    let attributes: Vec<_> = info.collect();
    let name = if matches!(
        language.as_str(),
        "yaml" | "containerfile" | "dockerfile" | "bash" | "sh"
    ) && attributes.iter().any(|attr| attr.starts_with("name="))
    {
        if attributes.len() != 1 {
            let message = if matches!(language.as_str(), "bash" | "sh") {
                "A named script fence must contain only its language and `name=script_name`."
            } else {
                "A named block fence must contain only its language and `name=block_name`."
            };
            return Err(user_error(message));
        }
        Some(attributes[0].strip_prefix("name=").unwrap())
    } else {
        None
    };
    let kind = match (language.as_str(), name) {
        ("yaml", Some("config")) => BlockKind::Config,
        ("yaml", Some(name)) => {
            return Err(user_error(format!(
                "Unknown YAML block name '{name}'. Use `yaml name=config` for module configuration."
            )));
        }
        ("containerfile" | "dockerfile", Some(name @ ("build" | "main" | "finalize"))) => {
            BlockKind::Containerfile(name.to_owned())
        }
        ("containerfile" | "dockerfile", Some(name)) => {
            return Err(user_error(format!(
                "Unknown Containerfile fragment name '{name}'. Use `build`, `main`, or `finalize`."
            )));
        }
        ("bash" | "sh", Some(name)) => {
            if name.is_empty()
                || !name
                    .bytes()
                    .all(|c| c.is_ascii_alphanumeric() || c == b'_' || c == b'-')
            {
                return Err(user_error(
                    "A script name must contain only ASCII letters, digits, underscores, or hyphens.",
                ));
            }
            BlockKind::Script {
                name: name.to_owned(),
                interpreter: language,
            }
        }
        _ => BlockKind::Documentation,
    };
    Ok(Some(Fence {
        marker,
        length,
        indent,
        kind,
        body: String::new(),
    }))
}

impl Fence {
    fn closes(&self, line: &str) -> bool {
        let trimmed = line.trim();
        let length = trimmed.chars().take_while(|c| *c == self.marker).count();
        length >= self.length && trimmed[length..].is_empty()
    }

    fn push(&mut self, line: &str) {
        // Remove only the indentation introduced by the opening Markdown fence.
        let indent = line
            .bytes()
            .take(self.indent)
            .take_while(|c| *c == b' ')
            .count();
        self.body.push_str(&line[indent..]);
        self.body.push('\n');
    }
}

pub fn parse(data: &str) -> eros::Result<ModuleData> {
    let mut fragments = Fragments::default();
    let mut config = None;
    let mut scripts = HashMap::new();
    let mut active: Option<Fence> = None;
    let mut has_fences = false;
    for line in data.lines() {
        if let Some(fence) = &mut active {
            if !fence.closes(line) {
                fence.push(line);
                continue;
            }
            let fence = active.take().unwrap();
            match fence.kind {
                BlockKind::Config => {
                    if config.replace(fence.body).is_some() {
                        return Err(user_error(
                            "Configuration block 'config' is declared more than once in a module.",
                        ));
                    }
                }
                BlockKind::Containerfile(name) => fragments.insert(&name, fence.body)?,
                BlockKind::Script { name, interpreter } => {
                    if scripts
                        .insert(
                            name.clone(),
                            Script {
                                interpreter,
                                body: fence.body,
                            },
                        )
                        .is_some()
                    {
                        return Err(user_error(format!(
                            "Script '{name}' is declared more than once in a module. Give each script a unique name."
                        )));
                    }
                }
                BlockKind::Documentation => {}
            }
        } else {
            active = opening_fence(line)?;
            has_fences |= active.is_some();
        }
    }
    if let Some(fence) = active {
        match fence.kind {
            BlockKind::Config => {
                return Err(user_error(
                    "A module has an unclosed YAML fenced block. Add its closing fence.",
                ));
            }
            BlockKind::Containerfile(name) => {
                return Err(user_error(format!(
                    "Containerfile fragment '{name}' has an unclosed fenced block. Add its closing fence."
                )));
            }
            BlockKind::Script { name, .. } => {
                return Err(user_error(format!(
                    "Script '{name}' has an unclosed fenced block. Add its closing fence."
                )));
            }
            BlockKind::Documentation => {}
        }
    }
    if fragments.is_empty() {
        if has_fences {
            return Err(user_error(
                "A Markdown module must have at least one named Containerfile block. Use `containerfile name=main`, `name=build`, or `name=finalize`.",
            ));
        }
        return Ok(ModuleData::plain(data.to_owned()));
    }
    Ok(ModuleData {
        fragments,
        config: config.unwrap_or_default(),
        scripts,
    })
}
