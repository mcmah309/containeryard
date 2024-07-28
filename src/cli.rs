use std::path::PathBuf;

use clap::{Parser, Subcommand};

#[derive(Parser)]
#[clap(name = "yard", author = "Henry McMahon", version = "0.1.0", about = "A declarative reusable decentralized approach for defining containers", long_about = None)]
pub struct Cli {
    #[clap(subcommand)]
    pub command: Commands,
}

#[derive(Subcommand)]
pub enum Commands {
    /// Build Containerfiles from a `yard.yaml` file.
    Build {
        /// Path to the `yard.yaml` file.
        #[clap(default_value = ".")]
        path: PathBuf,
    },
    /// Initialize a `yard.yaml` file.
    Init {
        /// Path to initialize the `yard.yaml` file.
        #[clap(default_value = ".")]
        path: PathBuf,
    },
    /// Updates all "commit" entries for each remote to the current "HEAD".
    Update {
        /// Path to the `yard.yaml` file.
        #[clap(default_value = ".")]
        path: PathBuf,
    },
}
