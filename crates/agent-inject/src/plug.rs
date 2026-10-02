//! `agent-inject plug` / `unplug`: install or remove the agent skills. The
//! skills are embedded at compile time, so an installed binary carries them
//! with no checkout. Ported from agent-notes.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use include_dir::{Dir, include_dir};

use crate::util::output;

static SKILLS: Dir<'_> = include_dir!("$CARGO_MANIFEST_DIR/../../skills");

/// The only folders `plug` and `unplug` create or delete under an agent's
/// skills root, so the skills of other tools there stay untouched. A test
/// keeps this in step with `skills/`.
const OWNED_SKILLS: &[&str] = &["inject-photo"];

// Ties this module to the content of `skills/` (see `build.rs`).
const _: &str = env!("AGENT_INJECT_EMBED_FINGERPRINT");

const SKIP: &[&str] = include!("embed_skip.rs");

#[derive(Clone, Copy, Debug, PartialEq, Eq, clap::ValueEnum)]
pub(crate) enum Agent {
    /// Claude Code: skills under `~/.claude/skills`.
    #[value(name = "claude-code", alias = "claude")]
    ClaudeCode,
    /// An agent that reads `~/.agents/skills`.
    Generic,
}

impl Agent {
    const ALL: [Agent; 2] = [Agent::ClaudeCode, Agent::Generic];

    fn label(self) -> &'static str {
        match self {
            Agent::ClaudeCode => "claude-code",
            Agent::Generic => "generic",
        }
    }

    /// Its presence means the agent is on this machine.
    fn agent_dir(self, home: &Path) -> PathBuf {
        match self {
            Agent::ClaudeCode => home.join(".claude"),
            Agent::Generic => home.join(".agents"),
        }
    }

    fn install_path(self, home: &Path) -> PathBuf {
        self.agent_dir(home).join("skills")
    }

    fn owned_skill_dirs(self, home: &Path) -> Vec<PathBuf> {
        let root = self.install_path(home);
        OWNED_SKILLS.iter().map(|name| root.join(name)).collect()
    }

    fn installed(self, home: &Path) -> bool {
        self.owned_skill_dirs(home).iter().any(|dir| dir.exists())
    }
}

/// Install the skills into `agents`, or into every agent found when empty.
///
/// # Errors
/// `$HOME` is unset, or a filesystem error.
pub(crate) fn plug(agents: &[Agent]) -> Result<()> {
    let home = home_dir()?;
    let mut acted = 0;
    let selected = select(agents, |agent| agent.agent_dir(&home).exists());
    for &agent in &selected {
        acted += usize::from(install(agent, &home)?);
    }
    finish("plugging", selected.len(), acted);
    Ok(())
}

/// Remove the skills from `agents`, or from every agent that has them when
/// empty.
///
/// # Errors
/// `$HOME` is unset, or a filesystem error.
pub(crate) fn unplug(agents: &[Agent]) -> Result<()> {
    let home = home_dir()?;
    let mut acted = 0;
    let selected = select(agents, |agent| agent.installed(&home));
    for &agent in &selected {
        acted += usize::from(remove(agent, &home)?);
    }
    finish("unplugging", selected.len(), acted);
    Ok(())
}

fn select(agents: &[Agent], default: impl Fn(Agent) -> bool) -> Vec<Agent> {
    if agents.is_empty() {
        return Agent::ALL
            .into_iter()
            .filter(|&agent| default(agent))
            .collect();
    }
    let mut out = Vec::new();
    for &agent in agents {
        if !out.contains(&agent) {
            out.push(agent);
        }
    }
    out
}

fn finish(gerund: &str, selected: usize, acted: usize) {
    if selected == 0 {
        output::warn("no agents selected; pass --agent claude-code|generic");
    } else {
        output::status("Finished", &format!("{gerund} inject · {acted} agent(s)"));
    }
}

/// Remove the old install, then write the embedded one. An agent that is not
/// on this machine is skipped, so an explicit `--agent` never creates
/// `~/.claude` where there is no Claude Code.
fn install(agent: Agent, home: &Path) -> Result<bool> {
    let path = agent.install_path(home);
    if !agent.agent_dir(home).exists() {
        output::status_warn("Skipping", &format!("{} (not detected)", agent.label()));
        return Ok(false);
    }
    output::status(
        "Plugging",
        &format!("{} ({})", agent.label(), output::home_path(&path)),
    );
    remove_owned(agent, home)?;
    write_dir(&SKILLS, &path)?;
    Ok(true)
}

fn remove(agent: Agent, home: &Path) -> Result<bool> {
    let path = agent.install_path(home);
    if !agent.installed(home) {
        output::status_warn(
            "Skipping",
            &format!(
                "{} (not present at {})",
                agent.label(),
                output::home_path(&path)
            ),
        );
        return Ok(false);
    }
    output::status(
        "Unplugging",
        &format!("{} ({})", agent.label(), output::home_path(&path)),
    );
    remove_owned(agent, home)?;
    Ok(true)
}

fn remove_owned(agent: Agent, home: &Path) -> Result<()> {
    for path in agent.owned_skill_dirs(home) {
        if path.is_symlink() || path.is_file() {
            std::fs::remove_file(&path).with_context(|| format!("remove {}", path.display()))?;
        } else if path.is_dir() {
            std::fs::remove_dir_all(&path).with_context(|| format!("remove {}", path.display()))?;
        }
    }
    Ok(())
}

fn write_dir(dir: &Dir<'_>, dest: &Path) -> Result<()> {
    std::fs::create_dir_all(dest).with_context(|| format!("create {}", dest.display()))?;
    for file in dir.files() {
        let name = file
            .path()
            .file_name()
            .expect("an embedded file has a name");
        if skipped(name) {
            continue;
        }
        let target = dest.join(name);
        std::fs::write(&target, file.contents())
            .with_context(|| format!("write {}", target.display()))?;
    }
    for sub in dir.dirs() {
        let name = sub.path().file_name().expect("an embedded dir has a name");
        if !skipped(name) {
            write_dir(sub, &dest.join(name))?;
        }
    }
    Ok(())
}

fn skipped(name: &std::ffi::OsStr) -> bool {
    SKIP.iter().any(|skip| name == *skip)
}

fn home_dir() -> Result<PathBuf> {
    std::env::var_os("HOME")
        .map(PathBuf::from)
        .context("`$HOME` is not set")
}

#[cfg(test)]
mod tests {
    use super::{Agent, OWNED_SKILLS, SKILLS, install, remove};

    #[test]
    fn every_owned_skill_is_embedded_with_a_skill_md() {
        for &name in OWNED_SKILLS {
            assert!(
                SKILLS.get_file(format!("{name}/SKILL.md")).is_some(),
                "missing embedded SKILL.md: {name}"
            );
        }
    }

    #[test]
    fn every_embedded_skill_is_owned() {
        // `plug` writes every embedded dir but `unplug` deletes only the owned
        // ones, so a skill missing here would install and never uninstall.
        for dir in SKILLS.dirs() {
            let name = dir.path().to_string_lossy();
            assert!(OWNED_SKILLS.contains(&name.as_ref()), "not owned: {name}");
        }
        assert_eq!(OWNED_SKILLS.len(), SKILLS.dirs().count());
    }

    #[test]
    fn plug_writes_the_skills_and_unplug_leaves_other_skills() {
        let home = tempfile::tempdir().unwrap();
        let skills = home.path().join(".claude/skills");
        std::fs::create_dir_all(skills.join("someone-else")).unwrap();

        assert!(install(Agent::ClaudeCode, home.path()).unwrap());
        let skill = std::fs::read_to_string(skills.join("inject-photo/SKILL.md")).unwrap();
        assert!(skill.starts_with("---\nname: inject-photo\n"), "{skill}");

        assert!(remove(Agent::ClaudeCode, home.path()).unwrap());
        assert!(!skills.join("inject-photo").exists());
        assert!(skills.join("someone-else").exists());
    }

    #[test]
    fn plug_skips_an_agent_that_is_not_installed() {
        let home = tempfile::tempdir().unwrap();
        assert!(!install(Agent::Generic, home.path()).unwrap());
        assert!(!home.path().join(".agents").exists());
    }
}
