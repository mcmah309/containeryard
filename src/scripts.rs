use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
};

use eros::Context;
use serde::Deserialize;
use tera::{Kwargs, State, Tera};

use crate::user_error::user_error;

#[derive(Debug, Clone)]
pub struct Script {
    pub interpreter: String,
    pub body: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ScriptArgs {
    name: String,
    #[serde(default)]
    template: bool,
    #[serde(default)]
    dedent: bool,
}

struct Expansion {
    marker: String,
    heredoc: String,
}

fn shell_quote(value: &str, _: Kwargs, _: &State) -> tera::TeraResult<String> {
    if value.contains('\0') {
        return Err(tera::Error::message(
            "Shell arguments cannot contain NUL bytes.",
        ));
    }
    Ok(format!("'{}'", value.replace('\'', "'\"'\"'")))
}

fn script_tera() -> Tera {
    let mut tera = Tera::default();
    tera.autoescape_on(Vec::<&str>::new());
    tera.register_filter("shell_quote", shell_quote);
    tera
}

fn dedent(body: &str) -> String {
    let prefix = body
        .lines()
        .filter(|line| !line.trim().is_empty())
        .map(|line| &line[..line.len() - line.trim_start_matches([' ', '\t']).len()])
        .reduce(|left, right| {
            let common = left
                .bytes()
                .zip(right.bytes())
                .take_while(|(a, b)| a == b)
                .count();
            &left[..common]
        })
        .unwrap_or_default();
    body.split_inclusive('\n')
        .map(|line| line.strip_prefix(prefix).unwrap_or(line))
        .collect()
}

fn prepare_script(
    kwargs: Kwargs,
    scripts: &HashMap<String, Script>,
    context: &tera::Context,
) -> eros::Result<String> {
    let args: ScriptArgs = kwargs.deserialize().map_err(|error| {
        user_error("script() requires a string `name` and accepts optional boolean `template` and `dedent` arguments.")
            .context(error.to_string())
    })?;
    let script = scripts.get(&args.name).ok_or_else(|| user_error(format!(
        "Script '{}' was not found in this module. Declare a named `bash` or `sh` fenced block.", args.name
    )))?;
    let mut body = if args.dedent {
        dedent(&script.body)
    } else {
        script.body.clone()
    };
    if args.template {
        body = script_tera()
            .render_str(&body, context, false)
            .with_user_context(|| {
                format!(
                    "Could not render script '{}'. Check its Tera syntax and module arguments.",
                    args.name
                )
            })?;
    }
    if body.contains('\0') {
        return Err(user_error(format!(
            "Script '{}' contains a NUL byte.",
            args.name
        )));
    }
    // Test the final body, including templated values, before choosing a delimiter.
    let mut index = 0;
    let delimiter = loop {
        let candidate = format!("YARD_SCRIPT_{index}");
        if !body.lines().any(|line| line == candidate) {
            break candidate;
        }
        index += 1;
    };
    if !body.ends_with('\n') {
        body.push('\n');
    }
    Ok(format!(
        "<<'{delimiter}' {}\n{body}{delimiter}",
        script.interpreter
    ))
}

/// Collect output-wide parser directives before adding generated comments or script bodies.
#[derive(Default)]
pub struct OutputDirectives {
    values: HashMap<String, String>,
    uses_scripts: bool,
}

impl OutputDirectives {
    fn extract(&mut self, fragment: &str) -> eros::Result<String> {
        let mut header = true;
        let mut content = String::new();
        for line in fragment.split_inclusive('\n') {
            let trimmed = line.trim();
            if header {
                if let Some(comment) = trimmed.strip_prefix('#') {
                    if let Some((key, value)) = comment.split_once('=') {
                        let key = key.trim().to_ascii_lowercase();
                        if matches!(key.as_str(), "syntax" | "escape" | "check") {
                            let value = value.trim();
                            if value.is_empty() {
                                return Err(user_error(format!(
                                    "Dockerfile directive '# {key}=' has no value."
                                )));
                            }
                            if let Some(previous) = self.values.get(&key)
                                && previous != value
                            {
                                return Err(user_error(format!(
                                    "Conflicting Dockerfile '{key}' directives: '{previous}' and '{value}'. Use one value per output."
                                )));
                            }
                            self.values.insert(key, value.to_owned());
                            continue;
                        }
                    }
                } else if !trimmed.is_empty() {
                    header = false;
                }
            }
            content.push_str(line);
        }
        Ok(content)
    }

    pub fn prepend(mut self, content: String) -> String {
        if self.uses_scripts {
            self.values
                .entry("syntax".to_owned())
                .or_insert_with(|| "docker/dockerfile:1".to_owned());
        }
        let mut header = String::new();
        for key in ["syntax", "escape", "check"] {
            if let Some(value) = self.values.get(key) {
                header.push_str(&format!("# {key}={value}\n"));
            }
        }
        if header.is_empty() {
            content
        } else {
            format!("{header}\n{content}")
        }
    }
}

pub fn render(
    template: &str,
    scripts: &HashMap<String, Script>,
    context: &tera::Context,
    directives: &mut OutputDirectives,
) -> eros::Result<String> {
    if template.contains('\0') {
        return Err(user_error("A Containerfile template contains a NUL byte."));
    }
    let mut tera = script_tera();
    let scripts = scripts.clone();
    let script_context = context.clone();
    let expansions = Arc::new(Mutex::new(Vec::<Expansion>::new()));
    let script_error = Arc::new(Mutex::new(None));
    let captured_expansions = Arc::clone(&expansions);
    let captured_error = Arc::clone(&script_error);
    tera.register_function(
        "script",
        move |kwargs: Kwargs, _: &State| -> tera::TeraResult<String> {
            match prepare_script(kwargs, &scripts, &script_context) {
                Ok(heredoc) => {
                    let mut expansions = captured_expansions.lock().unwrap();
                    let marker = format!("\0YARD_SCRIPT_{}\0", expansions.len());
                    expansions.push(Expansion {
                        marker: marker.clone(),
                        heredoc,
                    });
                    Ok(marker)
                }
                Err(error) => {
                    *captured_error.lock().unwrap() = Some(error);
                    Err(tera::Error::message("Could not expand script()."))
                }
            }
        },
    );
    let rendered = tera.render_str(template, context, false);
    if let Some(error) = script_error.lock().unwrap().take() {
        return Err(error);
    }
    let rendered = rendered.user_context(
        "Could not render a module template. Check its template syntax and supplied arguments.",
    )?;
    let rendered = directives.extract(&rendered)?;
    let expansions = expansions.lock().unwrap();
    let mut content = String::new();
    for line in rendered.split_inclusive('\n') {
        if let Some(expansion) = expansions
            .iter()
            .find(|expansion| line.contains(&expansion.marker))
        {
            let (prefix, suffix) = line.split_once(&expansion.marker).unwrap();
            let mut words = prefix.split_whitespace();
            if !words
                .next()
                .is_some_and(|word| word.eq_ignore_ascii_case("RUN"))
                || !words.all(|word| word.starts_with("--"))
                || !suffix.trim().is_empty()
            {
                return Err(user_error(
                    "Place script() after `RUN` and optional RUN flags, with no text after it.",
                ));
            }
            // Do not carry Markdown/Tera indentation or trailing spaces onto the delimiter.
            content.push_str(prefix.trim());
            content.push(' ');
            content.push_str(&expansion.heredoc);
            content.push('\n');
            directives.uses_scripts = true;
        } else {
            content.push_str(line);
        }
    }
    if content.contains('\0') {
        return Err(user_error(
            "A rendered Containerfile contains a NUL byte or an altered script() reference.",
        ));
    }
    Ok(content.trim().to_owned())
}
