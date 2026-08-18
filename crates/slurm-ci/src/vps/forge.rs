//! The VPS's one HTTP GET per run: `.slurm-ci.toml` at the resolved rev, never
//! at branch head (§2). The token rides a `curl -K -` config on stdin.

use anyhow::{Context, Result};

use crate::proc::{env_or, env_var, Exec};
use crate::spec::{JobSpec, CONFIG_PATH};

pub struct Forge {
    api: String,
    pub git_host: String,
    curl: String,
}

impl Forge {
    pub fn from_env() -> Result<Self> {
        Ok(Forge {
            api: env_var("CI_FORGEJO_API")?.trim_end_matches('/').to_owned(),
            git_host: env_var("CI_FORGEJO_GIT_HOST")?,
            curl: env_or("CI_CURL_BIN", "curl"),
        })
    }

    pub fn job_spec(&self, repo: &str, commit: &str, token: &str) -> Result<JobSpec> {
        let url = format!(
            "{}/api/v1/repos/{repo}/raw/{CONFIG_PATH}?ref={commit}",
            self.api
        );
        let curlrc = format!("header = \"Authorization: token {token}\"\nurl = \"{url}\"\n");
        let body = Exec::new(&self.curl)
            .args(["-sfS", "--max-time", "30", "-K", "-"])
            .stdin(curlrc)
            .output()
            .with_context(|| format!("fetch {CONFIG_PATH} at {commit}"))?;
        JobSpec::from_toml(&body)
    }
}
