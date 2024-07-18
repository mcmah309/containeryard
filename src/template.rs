use std::path::Path;



pub fn save_local_yard_file_as_template(path: &Path, template_name: String) {
    unimplemented!();
}

pub fn save_remote_yard_file_as_template(
    path: &Path,
    template_name: String,
    reference: String,
    url: String,
) {
    unimplemented!();
}

// fn main() -> Result<(), Box<dyn std::error::Error>> {
//     let url = "https://github.com/your/repo.git";
//     let repo_path = "/path/to/local/repo";

//     let repo = gix::prepare_clone(url, path);

//     println!("Repository cloned to: {:?}", repo.work_dir());

//     Ok(())
// }