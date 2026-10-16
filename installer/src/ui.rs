use crate::{
    catalog::Catalog,
    cli::Options,
    data::shell_join,
    plan::{Plan, display_relative},
};
use anyhow::Result;
use std::{
    collections::BTreeSet,
    io::{self, IsTerminal, Write},
    path::Path,
};

#[derive(Clone, Copy)]
pub struct Ui {
    pub color: bool,
    pub live: bool,
}

pub fn terminal() -> bool {
    io::stdin().is_terminal()
        && io::stdout().is_terminal()
        && std::env::var("TERM").is_ok_and(|term| term != "dumb")
}

pub fn clean(text: &str) -> String {
    text.chars().filter(|c| !c.is_control()).collect()
}

pub fn clip(text: &str, width: usize) -> String {
    let text = clean(text);
    if text.chars().count() > width {
        format!(
            "{}…",
            text.chars()
                .take(width.saturating_sub(1))
                .collect::<String>()
        )
    } else {
        text
    }
}

impl Ui {
    pub fn current() -> Self {
        let live = io::stdin().is_terminal() && io::stdout().is_terminal();
        Self {
            live,
            color: live
                && std::env::var("TERM").is_ok_and(|term| term != "dumb")
                && std::env::var_os("NO_COLOR").is_none()
                && std::env::var("CLICOLOR").as_deref() != Ok("0"),
        }
    }
    pub fn paint(self, code: &str, text: &str) -> String {
        let text = clean(text);
        if self.color {
            format!("\x1b[{code}m{text}\x1b[0m")
        } else {
            text
        }
    }
    pub fn muted(self, text: &str) -> String {
        self.paint("2", text)
    }
    pub fn accent(self, text: &str) -> String {
        self.paint("1;36", text)
    }
    pub fn hint(self, output: &mut impl Write, text: &str) -> Result<()> {
        writeln!(output, "    {}", self.muted(text))?;
        Ok(())
    }
    pub fn section(self, output: &mut impl Write, step: usize, title: &str) -> Result<()> {
        writeln!(
            output,
            "\n  {}  {}\n",
            self.accent(&format!("{step} / 4")),
            self.paint("1", title)
        )?;
        Ok(())
    }
    pub fn intro(self, project: &Path, output: &mut impl Write) -> Result<()> {
        writeln!(
            output,
            "\n  {}  {}",
            self.paint("1", "Xerrion dev tooling"),
            self.muted(concat!("v", env!("CARGO_PKG_VERSION")))
        )?;
        writeln!(output, "  {}", self.muted(&project.display().to_string()))?;
        writeln!(
            output,
            "\n  Choose your tools, then review the setup before applying it."
        )?;
        Ok(())
    }
    pub fn review(
        self,
        plan: &Plan,
        options: &Options,
        catalog: &Catalog,
        output: &mut impl Write,
    ) -> Result<()> {
        self.section(output, 4, "Review your setup")?;
        writeln!(output, "    {}", self.paint("1", "Tools and directories"))?;
        for id in &options.tools {
            let path = options.dirs.get(id).map(String::as_str).unwrap_or(".");
            writeln!(
                output,
                "    {:<26} {}",
                catalog.tools[id].title,
                self.muted(path)
            )?;
        }
        let created = plan.changes.values().filter(|c| c.before.is_none()).count();
        let removed = plan.changes.values().filter(|c| c.after.is_none()).count();
        let updated = plan.changes.len() - created - removed;
        writeln!(
            output,
            "\n    {}",
            self.paint(
                "1",
                &format!("Files: {created} new, {updated} updated, {removed} removed")
            )
        )?;
        for (path, change) in &plan.changes {
            let (label, color) = if change.after.is_none() {
                ("remove", "33")
            } else if change.before.is_none() {
                ("create", "32")
            } else {
                ("update", "36")
            };
            writeln!(
                output,
                "    {}  {}",
                self.paint(color, &format!("{label:<6}")),
                clean(&path.display().to_string())
            )?;
        }
        if plan.changes.is_empty() {
            self.hint(output, "Your configuration is already up to date.")?;
        }
        let preserved = plan
            .notes
            .iter()
            .filter(|note| note.starts_with("KEEP "))
            .count();
        if preserved > 0 {
            self.hint(output, &format!("{preserved} existing settings preserved."))?;
        }
        for note in plan
            .notes
            .iter()
            .filter(|note| note.starts_with("WARNING:"))
        {
            writeln!(output, "\n    {}", self.paint("33", note))?;
        }
        writeln!(output, "\n    {}", self.paint("1", "Installation commands"))?;
        if plan.commands.is_empty() {
            self.hint(output, "None. This setup only writes configuration files.")?;
        }
        for command in &plan.commands {
            writeln!(output, "    {}", clean(&shell_join(&command.argv)?))?;
            self.hint(
                output,
                &format!(
                    "in {}",
                    display_relative(&plan.project, &command.directory)?
                ),
            )?;
        }
        self.hint(output, "No changes have been applied yet.")?;
        Ok(())
    }
    pub fn details(self, plan: &Plan, output: &mut impl Write) -> Result<()> {
        writeln!(output, "\n    {}", self.paint("1", "Plan details"))?;
        for note in plan.notes.iter().collect::<BTreeSet<_>>() {
            writeln!(output, "    {}", clean(note))?;
        }
        writeln!(output)?;
        Ok(())
    }
    pub fn complete(self, plan: &Plan, applied: bool, output: &mut impl Write) -> Result<()> {
        if applied {
            writeln!(
                output,
                "\n  {}",
                self.paint(
                    "1;32",
                    &format!(
                        "Setup complete. Applied {} file changes.",
                        plan.changes.len()
                    )
                )
            )?;
            writeln!(
                output,
                "  Run mise run check from your project directory when you are ready.\n"
            )?;
        } else {
            writeln!(
                output,
                "\n  {}",
                self.accent("Preview finished. No changes applied.")
            )?;
            writeln!(
                output,
                "  Run the installer again whenever you are ready.\n"
            )?;
        }
        Ok(())
    }
}
