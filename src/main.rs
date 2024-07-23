#![allow(dead_code)]
#![allow(unused_variables)]
// todo remove above

mod build;
mod cli;
mod common;
mod git;
mod template;

use std::{env, path::Path, process::exit};

use build::build;
use clap::Parser;
use cli::{Cli, Commands, TemplateCommands};
use common::UserMessageError;
use template::{save_local_yard_file_as_template, save_remote_yard_file_as_template};
use tracing::error;

#[tokio::main]
async fn main() {
    let cli = Cli::parse();

    let result: anyhow::Result<()> = match cli.command {
        Commands::Build { path } => build(&path).await,
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
        let is_debug = env::var("CONTAINERYARD_DEBUG")
            .map(|v| v == "true")
            .unwrap_or(false);
        if is_debug {
            eprintln!("{}", error);
        } else {
            let mut user_error_message_count = 0;
            for err in error.chain() {
                if let Some(user_message_error) = err.downcast_ref::<UserMessageError>() {
                    eprintln!("{}", user_message_error.message);
                    user_error_message_count += 1;
                }
            }
            if user_error_message_count == 0 {
                error!("There should always be a user message");
                eprintln!("For more info, try again with environment variable `CONTAINERYARD_DEBUG=true`.")
            }
            eprintln!("Oops something went wrong.");
        }
        exit(1);
    };
}
