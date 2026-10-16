use crate::{
    catalog::Catalog,
    cli::Options,
    detection::DetectedStack,
    plan::{Plan, inside},
    project,
    selector::{self, Item},
    ui::{Ui, clean},
};
use anyhow::{Result, ensure};
use std::{
    fmt,
    io::{BufRead, Write},
};

#[derive(Debug)]
pub struct Cancelled;
impl fmt::Display for Cancelled {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Setup cancelled. No changes applied.")
    }
}
impl std::error::Error for Cancelled {}

fn ask(input: &mut impl BufRead, output: &mut impl Write, prompt: &str, ui: Ui) -> Result<String> {
    write!(output, "    {} {} ", ui.accent(">"), clean(prompt))?;
    output.flush()?;
    let mut line = String::new();
    ensure!(
        input.read_line(&mut line)? > 0,
        "Input ended; no changes applied"
    );
    if !ui.live {
        writeln!(output)?;
    }
    let line = line.trim();
    if matches!(line.to_ascii_lowercase().as_str(), "q" | "quit" | "cancel") {
        return Err(Cancelled.into());
    }
    Ok(line.into())
}

pub fn confirm(input: &mut impl BufRead, output: &mut impl Write, prompt: &str) -> Result<bool> {
    let ui = Ui::current();
    loop {
        let answer = ask(input, output, prompt, ui)?;
        match answer.to_ascii_lowercase().as_str() {
            "y" | "yes" => return Ok(true),
            "" | "n" | "no" => return Ok(false),
            _ => ui.hint(output, "Choose y or n. Enter keeps No; q cancels setup.")?,
        }
    }
}

fn parse_selection(text: &str, catalog: &Catalog, items: &[Item]) -> Result<Vec<String>> {
    if text.eq_ignore_ascii_case("all") {
        return Ok(items.iter().map(|item| item.id.clone()).collect());
    }
    let mut tools = vec![];
    for name in text
        .split(|c: char| c.is_whitespace() || c == ',')
        .filter(|s| !s.is_empty())
    {
        let id = if let Ok(index) = name.parse::<usize>() {
            items
                .get(index.wrapping_sub(1))
                .map(|item| item.id.as_str())
        } else {
            catalog.resolve(name)
        };
        let id = id
            .ok_or_else(|| {
                anyhow::anyhow!("Unknown selection: {name}. Use a number or a listed name.")
            })?
            .to_owned();
        if !tools.contains(&id) {
            tools.push(id);
        }
    }
    ensure!(!tools.is_empty(), "Choose at least one tool.");
    Ok(tools)
}

pub fn select(
    options: &mut Options,
    catalog: &Catalog,
    stack: &DetectedStack,
    input: &mut impl BufRead,
    output: &mut impl Write,
) -> Result<()> {
    let ui = Ui::current();
    ui.section(output, 1, "Choose your tooling")?;
    let defaults = if options.tools.is_empty() {
        stack.keys().cloned().collect::<Vec<_>>()
    } else {
        options.tools.clone()
    };
    let mut items = catalog
        .tools
        .iter()
        .map(|(id, tool)| {
            let hint = stack
                .get(id)
                .map(|found| {
                    let first = found.evidence[0].display();
                    if found.evidence.len() == 1 {
                        format!("detected: {first}")
                    } else {
                        format!("detected: {first} (+{})", found.evidence.len() - 1)
                    }
                })
                .unwrap_or_else(|| "optional".into());
            Item {
                id: id.clone(),
                title: tool.title.clone(),
                hint,
                selected: defaults.contains(id),
            }
        })
        .collect::<Vec<_>>();
    if stack.is_empty() {
        ui.hint(
            output,
            "No supported stack found. You can choose tooling manually.",
        )?;
    } else {
        ui.hint(output, "Detected tools are already selected.")?;
    }
    writeln!(output)?;
    options.tools = if let Some(selected) = selector::choose(&mut items, ui, input, output)? {
        selected
    } else {
        for (index, item) in items.iter().enumerate() {
            writeln!(
                output,
                "    {}  [{}] {:<24} {}",
                index + 1,
                if item.selected { "x" } else { " " },
                clean(&item.title),
                ui.muted(&item.hint)
            )?;
        }
        writeln!(output)?;
        ui.hint(
            output,
            "Use numbers or names, separated by spaces. Type all to select everything.",
        )?;
        ui.hint(
            output,
            "Enter keeps the selected tools. Type q to cancel at any prompt.",
        )?;
        loop {
            let text = ask(input, output, "Tooling:", ui)?;
            if text.is_empty() && !defaults.is_empty() {
                break defaults.clone();
            }
            match parse_selection(&text, catalog, &items) {
                Ok(tools) => break tools,
                Err(error) => ui.hint(output, &error.to_string())?,
            }
        }
    };
    let titles = options
        .tools
        .iter()
        .map(|id| catalog.tools[id].title.as_str())
        .collect::<Vec<_>>();
    writeln!(
        output,
        "    {} {}",
        ui.paint("32", "Selected:"),
        clean(&titles.join(", "))
    )?;

    ui.section(output, 2, "Working directories")?;
    ui.hint(
        output,
        "Paths are relative to your project. Enter keeps each suggestion.",
    )?;
    ui.hint(output, "Use . for the project root; type q to cancel.")?;
    writeln!(output)?;
    let root = options.project.canonicalize()?;
    let (existing, _) = project::existing_mise(&root)?;
    for id in &options.tools {
        let found = stack.get(id);
        let default = options
            .dirs
            .get(id)
            .cloned()
            .or_else(|| {
                existing["vars"][format!("{id}_dir")]
                    .as_str()
                    .map(String::from)
            })
            .or_else(|| {
                found
                    .filter(|f| f.directories.len() == 1)
                    .map(|f| f.directories[0].to_string_lossy().into_owned())
            });
        if let Some(found) = found.filter(|f| f.directories.len() > 1) {
            ui.hint(
                output,
                &format!(
                    "{} found in: {}",
                    catalog.tools[id].title,
                    found
                        .directories
                        .iter()
                        .map(|p| p.display().to_string())
                        .collect::<Vec<_>>()
                        .join(", ")
                ),
            )?;
        }
        let ambiguous = found.is_some_and(|f| f.directories.len() > 1) && default.is_none();
        let default = default.unwrap_or_else(|| if ambiguous { String::new() } else { ".".into() });
        loop {
            let text = ask(
                input,
                output,
                &format!("{} [{}]:", catalog.tools[id].title, default),
                ui,
            )?;
            let directory = if text.is_empty() {
                default.clone()
            } else {
                text
            };
            match inside(&root, &directory) {
                Ok(path) if !directory.is_empty() && path.is_dir() => {
                    if let Some(previous) = existing["vars"][format!("{id}_dir")].as_str()
                        && previous != directory
                    {
                        ui.hint(output, &format!("Your mise configuration sets {id}_dir to {previous}. Keep that path, or update the setting before running setup."))?;
                        continue;
                    }
                    options.dirs.insert(id.clone(), directory);
                    break;
                }
                Ok(_) => ui.hint(
                    output,
                    "That directory does not exist. Choose an existing project directory.",
                )?,
                Err(error) => ui.hint(output, &error.to_string())?,
            }
        }
    }
    ui.section(output, 3, "Installation preferences")?;
    ui.hint(
        output,
        "Choose No to write configuration without downloading tools.",
    )?;
    writeln!(output)?;
    if !options.install_tools {
        ui.hint(output, "mise can download tools and run project hooks.")?;
        options.install_tools = confirm(input, output, "Install selected tools with mise? [y/N]")?;
    } else {
        ui.hint(output, "Tool installation enabled by your command.")?;
    }
    if options.tools.iter().any(|id| {
        matches!(
            catalog.tools[id].adapter,
            crate::catalog::Adapter::Python | crate::catalog::Adapter::Javascript
        ) || !catalog.tools[id].install_dependencies.is_empty()
    }) {
        if !options.install_deps {
            writeln!(output)?;
            ui.hint(
                output,
                "Dependency sync can download packages and run build scripts.",
            )?;
            options.install_deps = confirm(input, output, "Sync project dependencies? [y/N]")?;
        } else {
            ui.hint(output, "Dependency sync enabled by your command.")?;
        }
    }
    Ok(())
}

pub fn approve(plan: &Plan, input: &mut impl BufRead, output: &mut impl Write) -> Result<bool> {
    let ui = Ui::current();
    writeln!(output)?;
    ui.hint(
        output,
        "Enter keeps this preview. Type details for all plan notes, or q to cancel.",
    )?;
    loop {
        let answer = ask(input, output, "Apply this setup? [y/N/details]", ui)?;
        match answer.to_ascii_lowercase().as_str() {
            "y" | "yes" => return Ok(true),
            "" | "n" | "no" => return Ok(false),
            "d" | "details" => ui.details(plan, output)?,
            _ => ui.hint(
                output,
                "Choose y to apply, n to keep the preview, or details to inspect the plan.",
            )?,
        }
    }
}
