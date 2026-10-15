use anyhow::Result;
use std::{collections::BTreeSet, process::ExitCode};
use xerrion_dev_tooling_installer::{
    build_plan, catalog::Catalog, cli, data::shell_join, plan::display_relative,
    templates_directory,
};
fn run() -> Result<()> {
    let args: Vec<_> = std::env::args_os().collect();
    let catalog = Catalog::load(&templates_directory(&args)?)?;
    let matches = cli::command(&catalog).get_matches_from(args);
    if matches.get_flag("list-tools") {
        for (id, tool) in &catalog.tools {
            println!("{id}: {}", tool.title);
        }
        return Ok(());
    }
    let options = cli::options(&matches, &catalog)?;
    let plan = build_plan(&options, &catalog)?;
    println!(
        "{}: {}",
        if options.apply {
            "APPLY"
        } else {
            "PREVIEW (no writes or installs)"
        },
        plan.project.display()
    );
    let mut seen = BTreeSet::new();
    for note in &plan.notes {
        if seen.insert(note) {
            println!("{note}");
        }
    }
    for (rel, change) in &plan.changes {
        let action = if change.after.is_none() {
            "REMOVE"
        } else if change.before.is_none() {
            "CREATE"
        } else {
            "UPDATE"
        };
        println!("{action} {}", rel.display());
    }
    for command in &plan.commands {
        println!(
            "PLAN RUN [{}] {}",
            display_relative(&plan.project, &command.directory)?,
            shell_join(&command.argv)?
        );
    }
    if options.apply {
        plan.apply()?;
        println!("Applied {} file changes.", plan.changes.len());
    } else {
        println!(
            "Rerun with --apply to perform this plan. No installation commands run without --apply."
        );
    }
    Ok(())
}
fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("Error: {error:#}");
            ExitCode::FAILURE
        }
    }
}
