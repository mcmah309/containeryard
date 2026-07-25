#![allow(unused_variables)]
// todo remove above

mod build;
mod cli;
mod common;
mod remote_resolvers;
mod init;
mod update;

use std::process::exit;

use build::{build, output_order};
use clap::Parser;
use cli::{Cli, Commands};
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

        tracing::subscriber::set_global_default(subscriber)
            .expect("setting default subscriber failed");
    }

    let cli = Cli::parse();

    let result: eros::Result<()> = async move {
        match cli.command {
            Commands::Build {
                path,
                do_not_refetch,
            } => build(&path, do_not_refetch).await,
            Commands::Outputs { path } => {
                for output_name in output_order(&path).await? {
                    println!("{output_name}");
                }
                Ok(())
            }
            Commands::Init { path } => init(&path).await,
            Commands::Update { path } => update(&path),
        }
    }
    .await;
    if let Err(error) = result {
        eprintln!("Oops something went wrong.\n");
        eprintln!("{:?}", error);
        if !is_debug {
            eprintln!(
                "\nFor more info, try again with environment variable `CONTAINERYARD_DEBUG` set to anything but `0`."
            );
        }
        exit(1);
    };
}
