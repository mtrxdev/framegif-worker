pub mod auto;
pub mod blob;
pub mod caps;
pub mod encode;
pub mod gif;
pub mod hmac_auth;
pub mod jobs;
pub mod probe;
pub mod types;

pub use gif::Gif;
pub use jobs::run_job;
pub use types::{CreateJobRequest, EncodeError, JobError};

pub fn resolve_bin(env_name: &str, fallback: &str) -> String {
    if let Ok(path) = std::env::var(env_name)
        && !path.is_empty()
        && std::path::Path::new(&path).is_file()
    {
        return path;
    }
    fallback.to_string()
}
