//! Real git repositories in the shapes the Phase 3 measurements found.
//!
//! git, unlike scoop, is on every machine this crate is developed on, so the
//! riskiest code in Phase 3 is tested against the real binary rather than
//! against a fake that can only be self-consistent.
#![allow(dead_code)]

use std::path::{Path, PathBuf};
use std::process::Command;

pub fn git(dir: &Path, args: &[&str]) -> String {
    let out = Command::new("git")
        .current_dir(dir)
        .args(args)
        .output()
        .unwrap_or_else(|e| panic!("git {args:?} in {}: {e}", dir.display()));
    assert!(
        out.status.success(),
        "git {args:?} in {} failed: {}",
        dir.display(),
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8_lossy(&out.stdout).to_string()
}

pub struct Fixture {
    pub home: tempfile::TempDir,
}

impl Fixture {
    pub fn new() -> Fixture {
        Fixture {
            home: tempfile::tempdir().unwrap(),
        }
    }
    pub fn scoop_root(&self) -> PathBuf {
        self.home.path().join("scoop")
    }
    pub fn bucket_dir(&self, bucket: &str) -> PathBuf {
        self.scoop_root().join("buckets").join(bucket)
    }

    /// An empty bucket repository with an identity configured.
    pub fn bucket(&self, name: &str) -> PathBuf {
        let dir = self.bucket_dir(name);
        std::fs::create_dir_all(dir.join("bucket")).unwrap();
        git(&dir, &["init", "-q", "-b", "main"]);
        git(&dir, &["config", "user.email", "t@example.invalid"]);
        git(&dir, &["config", "user.name", "t"]);
        git(&dir, &["config", "commit.gpgsign", "false"]);
        dir
    }

    /// Commit one manifest and return the sha.
    pub fn commit(&self, dir: &Path, file: &str, version: &str, url_tag: &str) -> String {
        std::fs::write(
            dir.join("bucket").join(file),
            format!("{{\n    \"version\": \"{version}\",\n    \"url\": \"https://example.invalid/{url_tag}.zip\"\n}}\n"),
        )
        .unwrap();
        git(dir, &["add", "-A"]);
        git(dir, &["commit", "-q", "-m", &format!("{file} {version}")]);
        git(dir, &["rev-parse", "HEAD"]).trim().to_string()
    }

    /// The blob for `<rev>:bucket/<file>` exactly as git stores it.
    pub fn blob(&self, dir: &Path, rev: &str, file: &str) -> String {
        git(dir, &["show", &format!("{rev}:bucket/{file}")])
    }
}

/// Section B of the measurements: a version that reached the bucket only on a
/// side branch whose change was superseded at merge time. `git log -- <path>`
/// cannot see it; `--full-history` can.
///
/// Returns `(side_commit_for_1_0_1, main_commit_for_1_0_2)`.
pub fn merged_bucket(f: &Fixture, name: &str) -> (String, String) {
    let dir = f.bucket(name);
    f.commit(&dir, "tool.json", "1.0.0", "v100");
    git(&dir, &["checkout", "-q", "-b", "side"]);
    let side = f.commit(&dir, "tool.json", "1.0.1", "side101");
    git(&dir, &["checkout", "-q", "main"]);
    let main = f.commit(&dir, "tool.json", "1.0.2", "main102");
    git(
        &dir,
        &[
            "merge",
            "-q",
            "--no-ff",
            "-X",
            "ours",
            "side",
            "-m",
            "merge side",
        ],
    );
    (side, main)
}

/// Section E: the bucket spells the file with different case at an older
/// commit. Built with plumbing and never checked out -- `git mv` cannot make
/// this on macOS or Windows, whose filesystems are case-insensitive, and the
/// first probe run measured nothing because it tried.
pub fn case_renamed_bucket(f: &Fixture, name: &str) -> (String, String) {
    let dir = f.bucket(name);
    let old_body = "{\n    \"version\": \"1.0.0\"\n}\n";
    let new_body = "{\n    \"version\": \"1.0.1\"\n}\n";

    let write_tree = |path: &str, body: &str| -> String {
        let sha = {
            let mut c = Command::new("git");
            c.current_dir(&dir).args(["hash-object", "-w", "--stdin"]);
            c.stdin(std::process::Stdio::piped());
            c.stdout(std::process::Stdio::piped());
            let mut child = c.spawn().unwrap();
            use std::io::Write;
            child
                .stdin
                .as_mut()
                .unwrap()
                .write_all(body.as_bytes())
                .unwrap();
            let out = child.wait_with_output().unwrap();
            String::from_utf8_lossy(&out.stdout).trim().to_string()
        };
        git(&dir, &["read-tree", "--empty"]);
        git(
            &dir,
            &[
                "update-index",
                "--add",
                "--cacheinfo",
                &format!("100644,{sha},{path}"),
            ],
        );
        git(&dir, &["write-tree"]).trim().to_string()
    };

    let t1 = write_tree("bucket/Tool.json", old_body);
    let c1 = git(&dir, &["commit-tree", &t1, "-m", "Tool 1.0.0"])
        .trim()
        .to_string();
    let t2 = write_tree("bucket/tool.json", new_body);
    let c2 = git(&dir, &["commit-tree", &t2, "-p", &c1, "-m", "tool 1.0.1"])
        .trim()
        .to_string();
    git(&dir, &["update-ref", "refs/heads/main", &c2]);
    (c1, c2)
}
