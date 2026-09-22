#![allow(unused_variables)]
// todo remove above

mod build;
mod cli;
mod common;
mod init;
mod remote_resolvers;
mod update;
mod user_error;

use std::process::exit;

use build::{build, output_order};
use clap::Parser;
use cli::{Cli, Commands};
use eros::Context;
use init::init;
use tracing::Level;
use tracing_subscriber::FmtSubscriber;
use update::update;

#[tokio::main]
async fn main() {
    let is_debug = common::is_debug();
    if is_debug {
        let subscriber = FmtSubscriber::builder()
            .with_max_level(Level::TRACE)
            .finish();

        if let Err(error) = tracing::subscriber::set_global_default(subscriber) {
            eprintln!("Warning: debug logging could not be enabled: {error}");
        }
    }

    let cli = Cli::parse();

    let result: eros::Result<()> = async move {
        match cli.command {
            Commands::Build {
                path,
                do_not_refetch,
                with_cache_busting,
            } => build(&path, do_not_refetch, with_cache_busting)
                .await
                .with_context(|| format!("Run `yard build` in '{}'", path.display()))
                .user_context(
                    "Could not build the Containerfiles. Check yard.yaml and its referenced modules, then try again.",
                ),
            Commands::Outputs { path } => {
                let output_names = output_order(&path)
                    .await
                    .with_context(|| format!("Run `yard outputs` in '{}'", path.display()))
                    .user_context(
                        "Could not list the configured outputs. Check that yard.yaml exists and is valid.",
                    )?;
                for output_name in output_names {
                    println!("{output_name}");
                }
                Ok(())
            }
            Commands::Init { path } => init(&path)
                .await
                .with_context(|| format!("Run `yard init` in '{}'", path.display()))
                .user_context(
                    "Could not initialize yard.yaml. Check that the destination exists and is writable.",
                ),
            Commands::Update { path } => update(&path)
                .with_context(|| format!("Run `yard update` in '{}'", path.display()))
                .user_context(
                    "Could not update remote commits. Check yard.yaml, your network connection, and Git credentials.",
                ),
        }
    }
    .await;
    if let Err(error) = result {
        eprintln!("Error: {}", user_error::format_user_error(&error));
        if is_debug {
            eprintln!("\nDeveloper diagnostics:\n{error:?}");
        } else {
            eprintln!("\nFor developer diagnostics, retry with `CONTAINERYARD_DEBUG=1`.");
        }
        exit(1);
    };
}
