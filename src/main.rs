#![allow(unused_variables)]
// todo remove above

mod build;
mod cli;
mod common;
mod git;
mod init;
mod update;

use std::{env, process::exit};

use build::build;
use clap::Parser;
use cli::{Cli, Commands};
use common::UserMessageError;
use init::init;
use tracing::{error, Level};
use tracing_subscriber::FmtSubscriber;
use update::update;

#[tokio::main]
async fn main() {
    let is_debug = env::var("CONTAINERYARD_DEBUG")
        .map(|v| v == "true")
        .unwrap_or(false);
    if is_debug {
        let subscriber = FmtSubscriber::builder()
            .with_max_level(Level::TRACE)
            .finish();

        tracing::subscriber::set_global_default(subscriber)
            .expect("setting default subscriber failed");
    }

    let cli = Cli::parse();

    let result: anyhow::Result<()> = match cli.command {
        Commands::Build { path } => build(&path).await,
        Commands::Init { path } => init(&path).await,
        Commands::Update { path } => update(&path),
    };
    if let Err(error) = result {
        if is_debug {
            eprintln!("{:?}", error);
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
            }
        }
        eprintln!("Oops something went wrong.");
        if !is_debug {
            eprintln!(
                "For more info, try again with environment variable `CONTAINERYARD_DEBUG=true`."
            );
        }
        exit(1);
    };
}
