//! Commands that only touch local files: `skill …` and `config …`.

use anyhow::Result;

use crate::cli::{ConfigCommand, SkillCommand, SkillInstallArgs};
use crate::config::{self, ServerSource, Settings, TokenSource};
use crate::output;
use crate::skill::{self, Outcome};

pub fn skill(command: SkillCommand) -> Result<()> {
    match command {
        SkillCommand::Install(args) => skill_install(args),
        SkillCommand::Show => output::print(skill::skill_md()),
        SkillCommand::Path => {
            let path = skill::claude_skill_dir(&skill::home_dir()?).join("SKILL.md");
            output::print(&format!("{}\n", path.display()))
        }
    }
}

fn skill_install(args: SkillInstallArgs) -> Result<()> {
    let mut dirs = Vec::new();
    if let Some(dir) = &args.dir {
        dirs.push(dir.clone());
    } else {
        let root = if args.project {
            std::env::current_dir()?
        } else {
            skill::home_dir()?
        };
        dirs.push(skill::claude_skill_dir(&root));
        if args.agents {
            dirs.push(skill::agents_skill_dir(&root));
        }
    }

    let done = skill::install(&dirs, args.force)?;
    let mut out = String::new();
    for (path, outcome) in &done {
        let verb = match outcome {
            Outcome::Wrote => "wrote    ",
            Outcome::Unchanged => "unchanged",
        };
        out.push_str(&format!("{verb} {}\n", path.display()));
    }
    let readers = if args.dir.is_some() {
        "point your agent at that directory"
    } else {
        "Claude Code, craze and opencode will pick it up in new sessions"
    };
    out.push_str(&format!("Installed the parcel skill; {readers}.\n"));
    output::print(&out)
}

pub fn config(mut settings: Settings, command: ConfigCommand) -> Result<()> {
    match command {
        ConfigCommand::Show => {
            let server_from = match settings.server_source {
                ServerSource::Flag => "from --server / PARCEL_SERVER",
                ServerSource::Profile => "from the profile",
                ServerSource::Default => "default",
            };
            let token = match settings.token_source {
                TokenSource::Env => "set (from PARCEL_TOKEN)".to_string(),
                TokenSource::Profile => "set (saved in the profile)".to_string(),
                TokenSource::None => match &settings.token_server_mismatch {
                    Some(other) => {
                        format!("not set for this server (profile is logged in to {other})")
                    }
                    None => "not set (run `parcel login`)".to_string(),
                },
            };
            let mut profiles: Vec<&str> = settings
                .config
                .profiles
                .keys()
                .map(String::as_str)
                .collect();
            if !profiles.contains(&settings.profile.as_str()) {
                profiles.push(&settings.profile);
            }
            profiles.sort_unstable();
            output::print(&format!(
                "profile:  {}\nserver:   {} ({server_from})\ntoken:    {token}\nconfig:   {}\nprofiles: {}\n",
                settings.profile,
                settings.server_display(),
                settings.store.path().display(),
                profiles.join(", ")
            ))
        }
        ConfigCommand::SetServer { url } => {
            let server = config::parse_server(&url)?;
            let display = config::server_string(&server);
            let mut profile = settings.profile_entry();
            let previous = profile
                .server
                .clone()
                .unwrap_or_else(|| config::DEFAULT_SERVER.to_string());
            let mut note = String::new();
            // A token only works on the server that issued it.
            if profile.token.is_some() && previous != display {
                profile.token = None;
                note = format!(
                    "Removed the saved token for {previous}; run `parcel login` to sign in to {display}.\n"
                );
            }
            profile.server = Some(display.clone());
            settings.save_profile(profile)?;
            output::print(&format!(
                "Profile {:?} now uses {display}\n{note}",
                settings.profile
            ))
        }
    }
}
