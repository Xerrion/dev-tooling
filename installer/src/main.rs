use anyhow::{Result, ensure};
use std::{
    collections::BTreeSet,
    io::{self, IsTerminal},
    process::ExitCode,
};
use xerrion_dev_tooling_installer::{
    build_plan,
    catalog::Catalog,
    cli,
    data::shell_join,
    detection, interactive,
    plan::display_relative,
    templates_directory,
    ui::{Ui, clean},
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
    let mut options = cli::options(&matches, &catalog)?;
    let interactive_mode = matches.get_flag("interactive")
        || (options.tools.is_empty()
            && !matches.get_flag("detect")
            && io::stdin().is_terminal()
            && io::stdout().is_terminal());
    let ui = Ui::current();
    if interactive_mode {
        interactive::configure_prompts(ui.color);
        ui.intro(&options.project.canonicalize()?, &mut io::stdout())?;
    }
    if options.tools.is_empty() || interactive_mode {
        ensure!(
            interactive_mode || matches.get_flag("detect"),
            "No tooling selected. Use --detect for automatic selection, --tools for explicit selection, or --interactive for prompts"
        );
        let stack = detection::detect(&options.project, &catalog)?;
        if stack.is_empty() && !interactive_mode {
            println!("No supported stack detected.");
        }
        for (id, found) in stack.iter().filter(|_| !interactive_mode) {
            let evidence = found
                .evidence
                .iter()
                .take(3)
                .map(|path| clean(&path.display().to_string()))
                .collect::<Vec<_>>();
            println!(
                "DETECTED {id}: {}{}",
                evidence.join(", "),
                if found.evidence.len() > 3 {
                    " (more matches)"
                } else {
                    ""
                }
            );
        }
        if interactive_mode {
            interactive::select(
                &mut options,
                &catalog,
                &stack,
                &mut io::stdin().lock(),
                &mut io::stdout(),
            )?;
        } else {
            options.tools = stack.keys().cloned().collect();
            ensure!(
                !options.tools.is_empty(),
                "No supported stack detected; use --tools to select explicitly"
            );
            detection::resolve_directories(&mut options, &stack)?;
        }
        detection::file_defaults(&mut options, &stack)?;
    }
    let plan = build_plan(&options, &catalog)?;
    if interactive_mode {
        ui.review(&plan, &options, &catalog, &mut io::stdout())?;
        if options.apply || matches.get_flag("dry-run") {
            ui.details(&plan, &mut io::stdout())?;
        }
    } else {
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
    }
    if interactive_mode && !options.apply && !matches.get_flag("dry-run") {
        options.apply = interactive::approve(&plan, &mut io::stdin().lock(), &mut io::stdout())?;
    }
    if options.apply {
        plan.apply()?;
    }
    if interactive_mode {
        ui.complete(&plan, options.apply, &mut io::stdout())?;
    } else if options.apply {
        println!("Applied {} file changes.", plan.changes.len());
    } else {
        println!(
            "Preview only. Rerun with --apply or confirm the plan in interactive setup to apply changes."
        );
    }
    Ok(())
}
fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            if error.is::<interactive::Cancelled>() {
                println!("\n  {}\n", Ui::current().muted(&error.to_string()));
                return ExitCode::SUCCESS;
            }
            if io::stderr().is_terminal() {
                eprintln!(
                    "\n  {} {error:#}\n",
                    Ui::current().paint("1;31", "Setup stopped:")
                );
            } else {
                eprintln!("Error: {error:#}");
            }
            ExitCode::FAILURE
        }
    }
}
