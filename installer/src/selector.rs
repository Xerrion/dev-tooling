//! Optional Unix terminal controls. Numbered prompts remain available everywhere.
use crate::ui::Ui;
#[cfg(unix)]
use crate::{interactive::Cancelled, ui::clip};
use anyhow::Result;
use std::io::{BufRead, Write};

pub struct Item {
    pub id: String,
    pub title: String,
    pub hint: String,
    pub selected: bool,
}

#[cfg(unix)]
struct TerminalMode(String);

#[cfg(unix)]
impl TerminalMode {
    fn enter() -> Option<Self> {
        use std::process::{Command, Stdio};
        let saved = Command::new("stty")
            .arg("-g")
            .stdin(Stdio::inherit())
            .output()
            .ok()?;
        if !saved.status.success() {
            return None;
        }
        let saved = String::from_utf8(saved.stdout).ok()?.trim().to_owned();
        let status = Command::new("stty")
            .args(["-icanon", "-echo", "-isig", "min", "0", "time", "1"])
            .stdin(Stdio::inherit())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .ok()?;
        if !status.success() {
            return None;
        }
        Some(Self(saved))
    }
}

#[cfg(unix)]
impl Drop for TerminalMode {
    fn drop(&mut self) {
        use std::process::{Command, Stdio};
        let _ = Command::new("stty")
            .arg(&self.0)
            .stdin(Stdio::inherit())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
        let mut output = std::io::stdout();
        let _ = write!(output, "\x1b[?25h");
        let _ = output.flush();
    }
}

#[cfg(unix)]
fn dimensions() -> (usize, usize) {
    use std::process::{Command, Stdio};
    let size = Command::new("stty")
        .arg("size")
        .stdin(Stdio::inherit())
        .output()
        .ok();
    let text = size.as_ref().map(|o| String::from_utf8_lossy(&o.stdout));
    let mut values = text
        .as_deref()
        .unwrap_or("")
        .split_whitespace()
        .filter_map(|s| s.parse::<usize>().ok());
    (
        values.next().filter(|n| *n > 0).unwrap_or(24),
        values.next().filter(|n| *n > 0).unwrap_or(80),
    )
}

#[cfg(unix)]
fn byte(input: &mut impl BufRead) -> Result<Option<u8>> {
    let mut data = [0];
    Ok(if input.read(&mut data)? == 0 {
        None
    } else {
        Some(data[0])
    })
}

#[cfg(unix)]
pub fn choose(
    items: &mut [Item],
    ui: Ui,
    input: &mut impl BufRead,
    output: &mut impl Write,
) -> Result<Option<Vec<String>>> {
    if !crate::ui::terminal() {
        return Ok(None);
    }
    let Some(mode) = TerminalMode::enter() else {
        return Ok(None);
    };
    let (rows, columns) = dimensions();
    if columns < 48 || rows < 10 {
        drop(mode);
        return Ok(None);
    }
    let visible = items.len().min(rows.saturating_sub(7).max(1));
    let mut focus = items.iter().position(|item| item.selected).unwrap_or(0);
    let mut rendered = 0;
    let mut empty = false;
    write!(output, "\x1b[?25l")?;
    loop {
        if rendered > 0 {
            write!(output, "\x1b[{rendered}A\r\x1b[J")?;
        }
        let start = focus.saturating_sub(visible - 1);
        for (index, item) in items.iter().enumerate().skip(start).take(visible) {
            let pointer = if index == focus { ">" } else { " " };
            let checkbox = if item.selected { "[x]" } else { "[ ]" };
            let title = format!("{pointer} {checkbox} {:<24}", clip(&item.title, 24));
            let title = if index == focus {
                ui.accent(&title)
            } else {
                title
            };
            let hint = clip(&item.hint, columns.saturating_sub(38));
            writeln!(output, "    {title}  {}", ui.muted(&hint))?;
        }
        let selected = items.iter().filter(|item| item.selected).count();
        writeln!(
            output,
            "\n    {}",
            if empty {
                ui.paint("33", "Choose at least one tool to continue.")
            } else {
                ui.muted(&format!(
                    "{selected} selected · {} of {}",
                    focus + 1,
                    items.len()
                ))
            }
        )?;
        writeln!(
            output,
            "    {}",
            ui.muted(&clip(
                "↑/↓ move · Space toggle · a all",
                columns.saturating_sub(5)
            ))
        )?;
        writeln!(output, "    {}", ui.muted("Enter continue · Esc cancel"))?;
        rendered = visible + 4;
        output.flush()?;
        let key = loop {
            if let Some(key) = byte(input)? {
                break key;
            }
        };
        match key {
            3 | 4 | b'q' => {
                write!(output, "\x1b[{rendered}A\r\x1b[J")?;
                return Err(Cancelled.into());
            }
            27 => match byte(input)? {
                Some(b'[' | b'O') => match byte(input)? {
                    Some(b'A') => focus = focus.checked_sub(1).unwrap_or(items.len() - 1),
                    Some(b'B') => focus = (focus + 1) % items.len(),
                    _ => (),
                },
                None => {
                    write!(output, "\x1b[{rendered}A\r\x1b[J")?;
                    return Err(Cancelled.into());
                }
                _ => (),
            },
            b'k' => focus = focus.checked_sub(1).unwrap_or(items.len() - 1),
            b'j' => focus = (focus + 1) % items.len(),
            b' ' => {
                items[focus].selected = !items[focus].selected;
                empty = false;
            }
            b'a' => {
                let selected = !items.iter().all(|item| item.selected);
                for item in items.iter_mut() {
                    item.selected = selected;
                }
                empty = false;
            }
            b'\r' | b'\n' => {
                let selected = items
                    .iter()
                    .filter(|item| item.selected)
                    .map(|item| item.id.clone())
                    .collect::<Vec<_>>();
                if !selected.is_empty() {
                    write!(output, "\x1b[{rendered}A\r\x1b[J")?;
                    drop(mode);
                    return Ok(Some(selected));
                }
                empty = true;
            }
            _ => (),
        }
    }
}

#[cfg(not(unix))]
pub fn choose(
    _items: &mut [Item],
    _ui: Ui,
    _input: &mut impl BufRead,
    _output: &mut impl Write,
) -> Result<Option<Vec<String>>> {
    Ok(None)
}
