mod cli;
mod common;
mod modules;
mod validate;

use std::path::Path;

use clap::Parser;
use cli::{Cli, Commands, TemplateCommands};
use common::UserMessageError;
use gix::{Remote, Repository};
use modules::resolve_modules;

fn main() {
    let cli = Cli::parse();

    let result: anyhow::Result<()> = match cli.command {
        Commands::Build { path } => {
            // todo parse yard.yaml and validate that all referenced modules are declared
            // todo download any missing remote modules and resolve paths to pass to below
            // todo resolve all modules (todo validate all required args are declared in yard.yaml)
            // todo resolve all args (env vars)
            // todo pass applied args to each template and collect
            // todo write template file
            Ok(())
        }
        Commands::Init { path, template } => {
            // todo check if template exists. If so use that. Otherwise use default
            Ok(())
        }
        Commands::Template { command } => {
            match command {
                TemplateCommands::Save {
                    path,
                    template,
                    remote,
                } => {
                    let template_name = template.unwrap_or_else(|| {
                        if path.as_path() == Path::new(".") {
                            let path =
                                std::env::current_dir().expect("Failed to get current directory");
                            return path
                                .parent()
                                .expect("Failed to get parent directory")
                                .file_name()
                                .unwrap()
                                .to_str()
                                .unwrap()
                                .to_string();
                        }
                        return path.file_name().unwrap().to_str().unwrap().to_string();
                    });
                    if remote.is_empty() {
                        save_local_yard_file_as_template(&path, template_name);
                    } else {
                        assert!(remote.len() == 2);
                        let ref_ = remote[0].to_string();
                        let url = remote[1].to_string();
                        save_remote_yard_file_as_template(&path, template_name, ref_, url);
                    }
                    Ok(())
                }
                TemplateCommands::List => {
                    // todo list templates
                    Ok(())
                }
                TemplateCommands::Delete { template } => {
                    // todo delete template with name
                    Ok(())
                }
            }
        }
    };
    if let Err(error) = result {
        for err in error.chain() {
            if let Some(user_message_error) = err.downcast_ref::<UserMessageError>() {
                eprint!("{}", user_message_error.message);
            }
        }
    };
}

fn save_local_yard_file_as_template(path: &Path, template_name: String) {
    unimplemented!();
}

fn save_remote_yard_file_as_template(
    path: &Path,
    template_name: String,
    reference: String,
    url: String,
) {
    unimplemented!();
}

// fn main2() -> Result<(), Box<dyn std::error::Error>> {
//     let url = "https://github.com/your/repo.git";
//     let repo_path = "/path/to/local/repo";

//     let repo = gix::prepare_clone(url, path);

//     println!("Repository cloned to: {:?}", repo.work_dir());

//     Ok(())
// }
