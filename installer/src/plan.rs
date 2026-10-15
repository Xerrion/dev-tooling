use anyhow::{Context, Result, bail, ensure};
use std::{
    collections::BTreeMap,
    fs,
    io::{self, Write},
    path::{Component, Path, PathBuf},
    process::Command,
};

pub const FRAGMENT: &str = ".mise/conf.d/tooling.toml";
pub const BACKUPS: &str = ".tooling-backups";
pub const START: &str = "<!-- dev-tooling:start -->";
pub const END: &str = "<!-- dev-tooling:end -->";
pub const AGGREGATE_START: &str = "# dev-tooling:aggregates:start";
pub const AGGREGATE_END: &str = "# dev-tooling:aggregates:end";

pub fn inside(root: &Path, relative: impl AsRef<Path>) -> Result<PathBuf> {
    let relative = relative.as_ref();
    let mut path = root.to_path_buf();
    for component in relative.components() {
        match component {
            Component::CurDir => (),
            Component::Normal(part) => {
                path.push(part);
                match fs::symlink_metadata(&path) {
                    Ok(meta) => ensure!(
                        !meta.file_type().is_symlink(),
                        "Refusing symlinked path: {}",
                        path.display()
                    ),
                    Err(error) if error.kind() == io::ErrorKind::NotFound => (),
                    Err(error) => return Err(error.into()),
                }
            }
            _ => bail!(
                "Expected a project-relative path without '..': {}",
                relative.display()
            ),
        }
    }
    Ok(path)
}

pub fn relative(root: &Path, path: &Path) -> Result<PathBuf> {
    Ok(path.strip_prefix(root)?.to_path_buf())
}
pub fn display_relative(root: &Path, path: &Path) -> Result<String> {
    let path = relative(root, path)?;
    Ok(if path.as_os_str().is_empty() {
        ".".into()
    } else {
        path.to_string_lossy().replace('\\', "/")
    })
}
pub fn read_optional(path: &Path) -> Result<Option<Vec<u8>>> {
    match fs::read(path) {
        Ok(bytes) => Ok(Some(bytes)),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error).with_context(|| format!("Cannot read {}", path.display())),
    }
}
pub fn read_text(path: &Path) -> Result<String> {
    Ok(String::from_utf8(read_optional(path)?.unwrap_or_default())?)
}

#[derive(Debug)]
pub struct Change {
    pub before: Option<Vec<u8>>,
    pub after: Option<Vec<u8>>,
}
#[derive(Debug)]
pub struct PlannedCommand {
    pub argv: Vec<String>,
    pub directory: PathBuf,
}
#[derive(Debug)]
pub struct Plan {
    pub project: PathBuf,
    pub changes: BTreeMap<PathBuf, Change>,
    pub notes: Vec<String>,
    pub commands: Vec<PlannedCommand>,
    pub dependency_backups: BTreeMap<PathBuf, Vec<u8>>,
}
impl Plan {
    pub fn new(project: PathBuf) -> Self {
        Self {
            project,
            changes: BTreeMap::new(),
            notes: vec![],
            commands: vec![],
            dependency_backups: BTreeMap::new(),
        }
    }
    pub fn stage(&mut self, rel: impl AsRef<Path>, text: impl Into<String>) -> Result<()> {
        let path = inside(&self.project, rel)?;
        let rel = relative(&self.project, &path)?;
        let before = read_optional(&path)?;
        let after = Some(text.into().into_bytes());
        if before != after {
            self.changes.insert(rel, Change { before, after });
        } else {
            self.changes.remove(&rel);
        }
        Ok(())
    }
    pub fn remove(&mut self, rel: impl AsRef<Path>) -> Result<()> {
        let path = inside(&self.project, rel)?;
        if let Some(bytes) = read_optional(&path)? {
            self.changes.insert(
                relative(&self.project, &path)?,
                Change {
                    before: Some(bytes),
                    after: None,
                },
            );
        }
        Ok(())
    }
    pub fn command(&mut self, argv: Vec<String>, directory: &Path) {
        self.commands.push(PlannedCommand {
            argv,
            directory: directory.to_path_buf(),
        });
    }
    pub fn planned_text(&self, rel: impl AsRef<Path>) -> Result<String> {
        let path = inside(&self.project, rel)?;
        if let Some(change) = self.changes.get(&relative(&self.project, &path)?) {
            return Ok(String::from_utf8(change.after.clone().unwrap_or_default())?);
        }
        read_text(&path)
    }
    pub fn apply(&self) -> Result<()> {
        self.apply_with(|command| {
            let (program, args) = command
                .argv
                .split_first()
                .context("Empty installation command")?;
            let status = Command::new(program)
                .args(args)
                .current_dir(&command.directory)
                .status()
                .with_context(|| {
                    format!("Cannot run {program} in {}", command.directory.display())
                })?;
            ensure!(
                status.success(),
                "Command failed ({status}): {}",
                crate::data::shell_join(&command.argv)?
            );
            Ok(())
        })
    }

    /// Apply files and backups before dispatching commands in plan order.
    /// The runner can replace external processes when verifying an installation.
    pub fn apply_with(&self, mut run: impl FnMut(&PlannedCommand) -> Result<()>) -> Result<()> {
        let backup_root = inside(&self.project, BACKUPS)?;
        ensure!(
            !backup_root.exists() || backup_root.is_dir(),
            "Expected a backup directory: {}",
            backup_root.display()
        );
        for (rel, change) in &self.changes {
            ensure!(
                read_optional(&inside(&self.project, rel)?)? == change.before,
                "File changed during planning; rerun: {}",
                rel.display()
            );
        }
        for (rel, before) in &self.dependency_backups {
            ensure!(
                read_optional(&inside(&self.project, rel)?)?.as_ref() == Some(before),
                "Dependency file changed during planning; rerun: {}",
                rel.display()
            );
        }
        let mut existing = self.dependency_backups.clone();
        for (rel, change) in &self.changes {
            if let Some(before) = &change.before {
                existing.insert(rel.clone(), before.clone());
            }
        }
        if !existing.is_empty() {
            fs::create_dir_all(&backup_root)?;
            let backup = tempfile::Builder::new()
                .prefix("run-")
                .tempdir_in(&backup_root)?
                .keep();
            for rel in existing.keys() {
                let destination = backup.join(rel);
                fs::create_dir_all(destination.parent().context("Backup has no parent")?)?;
                fs::copy(inside(&self.project, rel)?, &destination)?;
            }
            println!("Backups: {}", backup.display());
        }
        for (rel, change) in &self.changes {
            let path = inside(&self.project, rel)?;
            match &change.after {
                None => fs::remove_file(&path)?,
                Some(after) => {
                    let parent = path.parent().context("Destination has no parent")?;
                    fs::create_dir_all(parent)?;
                    let mut temporary = tempfile::NamedTempFile::new_in(parent)?;
                    temporary.write_all(after)?;
                    if path.exists() {
                        temporary
                            .as_file()
                            .set_permissions(fs::metadata(&path)?.permissions())?;
                    }
                    #[cfg(unix)]
                    if !path.exists() {
                        use std::os::unix::fs::PermissionsExt;
                        temporary
                            .as_file()
                            .set_permissions(fs::Permissions::from_mode(0o644))?;
                    }
                    temporary
                        .persist(&path)
                        .with_context(|| format!("Cannot replace {}", path.display()))?;
                }
            }
        }
        for command in &self.commands {
            run(command)?;
        }
        Ok(())
    }
}

pub fn update_managed(text: &str, block: &str, start: &str, end: &str) -> Result<String> {
    let starts = text.matches(start).count();
    ensure!(
        starts == text.matches(end).count() && starts <= 1,
        "Malformed or duplicate managed tooling markers"
    );
    if let (Some(first), Some(last)) = (text.find(start), text.find(end)) {
        ensure!(last >= first, "Reversed managed tooling markers");
        return Ok(format!(
            "{}{block}{}",
            &text[..first],
            &text[last + end.len()..]
        ));
    }
    if block.is_empty() {
        return Ok(text.into());
    }
    let separator = if text.is_empty() {
        ""
    } else if text.ends_with('\n') {
        "\n"
    } else {
        "\n\n"
    };
    Ok(format!("{text}{separator}{block}\n"))
}
