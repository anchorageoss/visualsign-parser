use std::path::PathBuf;

use anyhow::{Context, Result};
use clap::Parser;

use release_notes::mintlify::{self, DocsDir, Scope};
use release_notes::render::{Audience, Style, render};
use release_notes::sources::{Git, collect, gh_repo};

/// Draft a release announcement: changes grouped by chain, the chains enabled in the
/// release, integrator notes and contributors. Reads git and `gh`; posts nothing.
#[derive(Parser)]
#[command(name = "release-notes")]
struct Args {
    /// Release tag to announce. Defaults to the newest tag at HEAD.
    #[arg(long)]
    to: Option<String>,

    /// Previous release tag. Defaults to the tag before --to.
    #[arg(long)]
    from: Option<String>,

    /// GitHub repo as owner/name. Defaults to the repo `gh` resolves for --repo-dir.
    #[arg(long)]
    repo: Option<String>,

    /// Checkout to read git history from.
    #[arg(long, default_value = ".")]
    repo_dir: PathBuf,

    #[arg(long, value_enum, default_value = "slack")]
    format: Format,

    /// `public` is safe for shared channels; `internal` adds tooling/infra and co-authors.
    #[arg(long, value_enum, default_value = "public")]
    audience: Audience,

    /// Text appended verbatim (e.g. deploy details). Keep this file outside the repo.
    #[arg(long)]
    footer_file: Option<PathBuf>,

    /// Instead of printing, write the release into the Mintlify docs dir (changelog
    /// entries, and a blog post for each newly enabled chain). Always the public view.
    #[arg(long, value_name = "DOCS_DIR", conflicts_with_all = ["format", "audience", "footer_file"])]
    write_mintlify: Option<PathBuf>,

    /// With --write-mintlify: write only if the release enables a new chain, and only
    /// that chain's pages.
    #[arg(long, requires = "write_mintlify")]
    new_chains_only: bool,
}

#[derive(Clone, Copy, clap::ValueEnum)]
enum Format {
    Slack,
    Markdown,
    /// The collected data, for scripts that build their own message.
    Json,
}

fn main() -> Result<()> {
    let args = Args::parse();
    let git = Git {
        dir: &args.repo_dir,
    };

    let to = match args.to {
        Some(t) => t,
        None => git.latest_tag("HEAD")?,
    };
    let from = match args.from {
        Some(f) => f,
        None => git
            .latest_tag(&format!("{to}^"))
            .with_context(|| format!("no tag before {to}; pass --from"))?,
    };
    let repo = match args.repo {
        Some(r) => r,
        None => gh_repo(&args.repo_dir)?,
    };
    let footer = args
        .footer_file
        .map(|p| std::fs::read_to_string(&p).with_context(|| format!("read {}", p.display())))
        .transpose()?;

    let release = collect(&git, &repo, &from, &to)?;

    if let Some(docs_dir) = &args.write_mintlify {
        let scope = if args.new_chains_only {
            Scope::NewChainsOnly
        } else {
            Scope::All
        };
        let plan = mintlify::plan(&release, &git.date(&to)?, scope, &DocsDir(docs_dir))?;
        mintlify::apply(docs_dir, &plan)?;
        if plan.is_empty() {
            eprintln!("nothing to write for {from}..{to}");
        }
        for path in plan.keys() {
            println!("{}", docs_dir.join(path).display());
        }
        return Ok(());
    }

    let out = match args.format {
        Format::Json => serde_json::to_string_pretty(&release.visible_to(args.audience))? + "\n",
        Format::Slack => render(&release, Style::Slack, args.audience, footer.as_deref()),
        Format::Markdown => render(&release, Style::Markdown, args.audience, footer.as_deref()),
    };
    print!("{out}");
    Ok(())
}
