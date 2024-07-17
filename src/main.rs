use std::path::Path;

use clap::Parser;
use cli::{Cli, Commands};
use gix::{Remote, Repository};

mod cli;

fn main() {
    let cli = Cli::parse();

    match cli.command {
        Commands::Build { path } => {
            println!("Building Containerfiles from {}", path);
        },
        Commands::Init { path, template } => {
            match template {
                Some(t) => println!("Initializing {} with template {}", path, t),
                None => println!("Initializing {}", path),
            }
        },
        Commands::Save { path, template , remote} => {
            let template_name = template.unwrap_or_else(|| {
                let path = std::env::current_dir().expect("Failed to get current directory");
                path.parent().map(|e| e.file_name()).flatten().expect("Failed to get current directory name").to_str().expect("Failed to convert to string").to_string()
            });
            if remote.is_empty() {
                save_local_yard_file_as_template(path, template_name);
            }
            else {
                assert!(remote.len() == 2);
                let ref_ = remote[0].to_string();
                let url = remote[1].to_string();
                save_remote_yard_file_as_template(path, template_name, ref_, url);
            }
        },
        Commands::List { templates } => {
            // Handle listing available templates
            if templates {
                println!("Listing templates");
            }
        },
        Commands::Delete { template } => {
            // Handle deleting a template
            println!("Deleting template {}", template);
        },
    }
}

fn save_local_yard_file_as_template<P: AsRef<Path>>(path: P, template_name: String) {
    let path = path.as_ref();
}

fn save_remote_yard_file_as_template<P: AsRef<Path>>(path: P, template_name: String, reference: String, url: String) {
    let path = path.as_ref();
}

// fn main2() -> Result<(), Box<dyn std::error::Error>> {
//     let url = "https://github.com/your/repo.git";
//     let repo_path = "/path/to/local/repo";
    
//     let repo = gix::prepare_clone(url, path);

//     println!("Repository cloned to: {:?}", repo.work_dir());

//     Ok(())
// }