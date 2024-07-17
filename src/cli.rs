use clap::{Parser, Subcommand};

#[derive(Parser)]
#[clap(name = "yard", author = "Henry McMahon", version = "0.1.0", about = "A declarative reusable decentralized approach for defining containers", long_about = None)]
pub struct Cli {
    #[clap(subcommand)]
    pub command: Commands,
}

#[derive(Subcommand)]
pub enum Commands {
    /// Build Containerfiles from a `yard.yaml` file
    Build {
        /// Path to the `yard.yaml` file
        #[clap(default_value = ".")]
        path: String,
    },
    /// Initialize a `yard.yaml` file
    Init {
        /// Path to initialize the `yard.yaml` file
        #[clap(default_value = ".")]
        path: String,
        /// Template to use for initialization
        #[clap(short, long)]
        template: Option<String>,
    },
    /// Save the current `yard.yaml` file as a template
    Save {
        /// Path to the `yard.yaml` file
        #[clap(default_value = ".")]
        path: String,
        /// Name of the template to save as
        #[clap(short, long)]
        template: Option<String>,
        /// Remote repository.
        #[clap(short, long, num_args = 2, value_names = ["REF", "REPO_URL"])]
        remote: Vec<String>,
    },
    /// List
    List {
        /// List saved templates
        #[clap(short, long,)] // conflicts_with = "templates", requires = "templates"
        templates: bool,
    },
    /// Delete a template
    Delete {
        /// Name of the template to delete
        #[clap(short, long)]
        template: String,
    },
}