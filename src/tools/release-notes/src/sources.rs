//! Read the release range from local git and PR metadata from GitHub via `gh`.
//! Nothing here is specific to a deployment; everything comes from the repo and its PRs.

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;
use std::process::Command;

use anyhow::{Context, Result, bail};
use serde::Deserialize;

use crate::chains::{
    ChainSurface, crates_from_dir_listing, parse_chain_features, parse_default_features,
    parse_proto_chains,
};
use crate::entries::{
    CoAuthor, co_author_trailers, display_line, integrator_note, is_bot, is_breaking,
    pr_number_from_subject, sections_for,
};
use crate::render::{Entry, Release};

const CHAIN_PARSERS_DIR: &str = "src/chain_parsers";
const APP_CARGO_TOML: &str = "src/parser/app/Cargo.toml";
const APP_CONTAINERFILE: &str = "images/parser_app/Containerfile";
const PROTO: &str = "proto/parser/parser.proto";

pub struct Git<'a> {
    pub dir: &'a Path,
}

impl Git<'_> {
    pub fn run(&self, args: &[&str]) -> Result<String> {
        run(
            Command::new("git").arg("-C").arg(self.dir).args(args),
            "git",
            args,
        )
    }

    /// The newest tag at or before `rev`.
    pub fn latest_tag(&self, rev: &str) -> Result<String> {
        Ok(self
            .run(&["describe", "--tags", "--abbrev=0", rev])?
            .trim()
            .to_string())
    }

    /// File contents at a ref; empty when the file doesn't exist there.
    pub fn show(&self, rev: &str, path: &str) -> String {
        self.run(&["show", &format!("{rev}:{path}")])
            .unwrap_or_default()
    }

    pub fn chain_surface(&self, rev: &str) -> ChainSurface {
        ChainSurface {
            crates: crates_from_dir_listing(
                &self
                    .run(&[
                        "ls-tree",
                        "--name-only",
                        &format!("{rev}:{CHAIN_PARSERS_DIR}"),
                    ])
                    .unwrap_or_default(),
            ),
            app_default: parse_default_features(&self.show(rev, APP_CARGO_TOML)),
            image_features: parse_chain_features(&self.show(rev, APP_CONTAINERFILE)),
            proto: parse_proto_chains(&self.show(rev, PROTO)),
        }
    }

    /// First-parent commits in `from..to`, oldest first, as (sha, subject).
    pub fn commits(&self, from: &str, to: &str) -> Result<Vec<(String, String)>> {
        let out = self.run(&[
            "log",
            "--first-parent",
            "--reverse",
            "--format=%H%x1f%s",
            &format!("{from}..{to}"),
        ])?;
        Ok(out
            .lines()
            .filter_map(|l| l.split_once('\x1f'))
            .map(|(sha, subject)| (sha.to_string(), subject.to_string()))
            .collect())
    }

    /// Files a first-parent commit changed (works for squash and merge commits).
    pub fn files(&self, sha: &str) -> Result<Vec<String>> {
        let out = self.run(&["diff", "--name-only", &format!("{sha}^"), sha])?;
        Ok(out.lines().map(str::to_string).collect())
    }

    pub fn message(&self, sha: &str) -> Result<String> {
        self.run(&["log", "-1", "--format=%B", sha])
    }
}

#[derive(Debug, Deserialize)]
pub struct PullRequest {
    pub title: String,
    pub url: String,
    pub body: String,
    pub author: Author,
    pub labels: Vec<Label>,
}

#[derive(Debug, Deserialize)]
pub struct Author {
    pub login: String,
    #[serde(default)]
    pub is_bot: bool,
}

#[derive(Debug, Deserialize)]
pub struct Label {
    pub name: String,
}

pub fn gh_repo(dir: &Path) -> Result<String> {
    let args = [
        "repo",
        "view",
        "--json",
        "nameWithOwner",
        "--jq",
        ".nameWithOwner",
    ];
    Ok(
        run(Command::new("gh").current_dir(dir).args(args), "gh", &args)?
            .trim()
            .to_string(),
    )
}

pub fn gh_pr(repo: &str, number: u64) -> Result<PullRequest> {
    let n = number.to_string();
    let args = [
        "pr",
        "view",
        &n,
        "--repo",
        repo,
        "--json",
        "title,url,body,author,labels",
    ];
    let out = run(Command::new("gh").args(args), "gh", &args)?;
    serde_json::from_str(&out).with_context(|| format!("parse gh output for PR #{number}"))
}

/// Everything the announcement needs for `from..to`.
pub fn collect(git: &Git<'_>, repo: &str, from: &str, to: &str) -> Result<Release> {
    let at = git.chain_surface(to);
    let previous = git.chain_surface(from);
    let known: BTreeSet<String> = at
        .crates
        .iter()
        .filter(|c| *c != "unspecified")
        .cloned()
        .collect();

    let mut logins = Logins::default();
    let mut entries = Vec::new();
    for (sha, subject) in git.commits(from, to)? {
        let Some(number) = pr_number_from_subject(&subject) else {
            eprintln!(
                "skipping {} (no PR number): {subject}",
                &sha[..sha.len().min(10)]
            );
            continue;
        };
        let pr = gh_pr(repo, number)?;
        let files = git.files(&sha)?;
        let labels: Vec<String> = pr.labels.into_iter().map(|l| l.name).collect();
        let author = if pr.author.is_bot || is_bot(&pr.author.login, "") {
            String::new()
        } else {
            pr.author.login
        };
        entries.push(Entry {
            number,
            url: pr.url,
            line: display_line(&pr.title, &pr.body),
            co_authors: {
                let mut people: Vec<String> = Vec::new();
                for c in co_author_trailers(&git.message(&sha)?) {
                    let who = logins.resolve(repo, &c);
                    if !is_bot(&who, "") && who != author && !people.contains(&who) {
                        people.push(who);
                    }
                }
                people
            },
            author,
            sections: sections_for(&pr.title, &files, &known),
            breaking: is_breaking(&pr.title, &labels, &pr.body),
            integrator_note: integrator_note(&pr.body),
        });
    }

    Ok(Release {
        repo: repo.to_string(),
        from: from.to_string(),
        to: to.to_string(),
        chains: crate::chains::rows(&at, &previous),
        entries,
    })
}

/// Co-author email -> GitHub login, cached per email.
#[derive(Default)]
struct Logins {
    cache: BTreeMap<String, Option<String>>,
}

impl Logins {
    /// The co-author's GitHub login if it can be found, otherwise their name. A
    /// non-noreply email is looked up through the repo's commits by that author.
    fn resolve(&mut self, repo: &str, c: &CoAuthor) -> String {
        if let Some(login) = c.noreply_login() {
            return login;
        }
        if c.email.is_empty() {
            return c.name.clone();
        }
        let key = c.email.to_ascii_lowercase();
        let login = self
            .cache
            .entry(key)
            .or_insert_with(|| gh_login_for_email(repo, &c.email))
            .clone();
        login.unwrap_or_else(|| c.name.clone())
    }
}

fn gh_login_for_email(repo: &str, email: &str) -> Option<String> {
    let email: String = email
        .chars()
        .map(|c| match c {
            '+' => "%2B".to_string(),
            '&' => "%26".to_string(),
            '#' => "%23".to_string(),
            c => c.to_string(),
        })
        .collect();
    let path = format!("repos/{repo}/commits?author={email}&per_page=1");
    let args = ["api", &path, "--jq", ".[0].author.login // empty"];
    let login = run(Command::new("gh").args(args), "gh", &args).ok()?;
    let login = login.trim();
    (!login.is_empty()).then(|| login.to_string())
}

fn run(cmd: &mut Command, program: &str, args: &[&str]) -> Result<String> {
    let out = cmd
        .output()
        .with_context(|| format!("failed to run {program} (is it installed and on PATH?)"))?;
    if !out.status.success() {
        bail!(
            "{program} {} failed: {}",
            args.join(" "),
            String::from_utf8_lossy(&out.stderr).trim()
        );
    }
    String::from_utf8(out.stdout).with_context(|| format!("{program} output is not UTF-8"))
}
