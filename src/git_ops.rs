use anyhow::{Context, Result};
use colored::Colorize;
use git2::{Repository, Status, StatusOptions};
use std::process::Command;
use std::time::UNIX_EPOCH;

#[allow(dead_code)]
pub struct GitRepo {
    repo: Repository,
    path: std::path::PathBuf,
}

#[allow(dead_code)]
impl GitRepo {
    pub fn open(path: &std::path::Path) -> Result<Self> {
        let repo = Repository::open(path)
            .with_context(|| format!("Failed to open git repository at {:?}", path))?;
        Ok(Self {
            repo,
            path: path.to_path_buf(),
        })
    }

    pub fn init(path: &std::path::Path) -> Result<Self> {
        let repo = Repository::init(path)
            .with_context(|| format!("Failed to initialize git repository at {:?}", path))?;
        Ok(Self {
            repo,
            path: path.to_path_buf(),
        })
    }

    pub fn is_repo(path: &std::path::Path) -> bool {
        Repository::open(path).is_ok()
    }

    pub fn get_common_dir(&self) -> Result<std::path::PathBuf> {
        Ok(self.repo.path().to_path_buf())
    }

    pub fn status(&self) -> Result<String> {
        let mut opts = StatusOptions::new();
        opts.include_untracked(true);

        let statuses = self.repo.statuses(Some(&mut opts))?;

        if statuses.is_empty() {
            Ok("No changes to commit".to_string())
        } else {
            let mut result = String::new();
            for entry in statuses.iter() {
                if let Some(path) = entry.path() {
                    let status = entry.status();
                    let status_text = match status {
                        s if s.contains(Status::WT_NEW) => "untracked".to_string(),
                        s if s.contains(Status::WT_MODIFIED) => "modified".to_string(),
                        s if s.contains(Status::INDEX_NEW) => "added".to_string(),
                        s if s.contains(Status::INDEX_DELETED) => "deleted".to_string(),
                        s if s.contains(Status::WT_DELETED) => "deleted".to_string(),
                        _ => format!("{:?}", status),
                    };
                    result.push_str(&format!("  {:<10} {}\n", status_text, path));
                }
            }
            Ok(result)
        }
    }

    pub fn has_changes(&self) -> Result<bool> {
        let mut opts = StatusOptions::new();
        opts.include_untracked(true);
        let statuses = self.repo.statuses(Some(&mut opts))?;
        Ok(!statuses.is_empty())
    }

    pub fn add_all(&self) -> Result<()> {
        let mut index = self.repo.index()?;
        index.add_all(["*"].iter(), git2::IndexAddOption::DEFAULT, None)?;
        index.write()?;
        Ok(())
    }

    pub fn commit(&self, message: &str) -> Result<()> {
        let tree_id = {
            let mut index = self.repo.index()?;
            index.write()?;
            index.write_tree()?
        };

        let tree = self.repo.find_tree(tree_id)?;

        let sig = self.repo.signature()?;

        let head_commit = self
            .repo
            .head()
            .ok()
            .and_then(|head| head.target())
            .and_then(|oid| self.repo.find_commit(oid).ok());

        let parents: Vec<_> = head_commit.iter().collect();

        let _commit_id = self.repo.commit(
            Some("HEAD"),
            &sig,
            &sig,
            message,
            &tree,
            parents.as_slice(),
        )?;

        println!("{}", "✅ Committed successfully".green());
        Ok(())
    }

    pub fn get_last_commit(&self) -> Result<String> {
        let head = self.repo.head().context("Repository has no HEAD commit")?;

        let commit = self
            .repo
            .find_commit(head.target().context("HEAD has no target")?)
            .context("Could not find HEAD commit")?;

        let time = commit.time();
        let seconds = time.seconds().unsigned_abs();
        let datetime = UNIX_EPOCH + std::time::Duration::from_secs(seconds);
        let timestamp = chrono_humanize::HumanTime::from(datetime);

        Ok(format!(
            "{} {} ({})",
            commit.id(),
            commit.summary().unwrap_or("(no message)"),
            timestamp.to_text_en(
                chrono_humanize::Accuracy::Rough,
                chrono_humanize::Tense::Present
            )
        ))
    }

    pub fn remote_url(&self) -> Option<String> {
        let remote = self.repo.find_remote("origin").ok()?;
        remote.url().map(String::from)
    }

    /// Pull with rebase using git command (git2 doesn't support rebase well)
    pub fn pull_rebase(&self) -> Result<()> {
        let output = Command::new("git")
            .args(["pull", "--rebase"])
            .current_dir(&self.path)
            .output()
            .context("Failed to execute git pull --rebase")?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            if stderr.contains("CONFLICT")
                || stderr.contains("Automatic merge failed")
                || stderr.contains("Patch failed")
            {
                return Err(anyhow::anyhow!(
                    "Merge conflict detected. Please resolve conflicts manually in {:?}",
                    self.path
                ));
            }
            return Err(anyhow::anyhow!("git pull --rebase failed: {}", stderr));
        }

        Ok(())
    }

    pub fn push(&self) -> Result<()> {
        let output = Command::new("git")
            .args(["push"])
            .current_dir(&self.path)
            .output()
            .context("Failed to execute git push")?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(anyhow::anyhow!("git push failed: {}", stderr));
        }

        println!("{}", "✅ Pushed to remote".green());
        Ok(())
    }
}
