use std::io::{self, Write};

use crossterm::cursor;
use crossterm::event::{self, Event, KeyCode, KeyModifiers};
use crossterm::style::{self, Stylize};
use crossterm::terminal;
use crossterm::ExecutableCommand;

/// Interactive picker: arrow keys to navigate, number to jump, Enter to confirm, Esc to abort.
/// Returns the selected index, or None if aborted.
pub fn pick(title: &str, options: &[String]) -> anyhow::Result<Option<usize>> {
    if options.is_empty() {
        return Ok(None);
    }
    if options.len() == 1 {
        return Ok(Some(0));
    }

    let mut stdout = io::stdout();
    terminal::enable_raw_mode()?;

    let result = run_picker(&mut stdout, title, options);

    terminal::disable_raw_mode()?;

    // Clear the picker output
    stdout.execute(cursor::MoveToColumn(0))?;

    result
}

fn run_picker(
    stdout: &mut io::Stdout,
    title: &str,
    options: &[String],
) -> anyhow::Result<Option<usize>> {
    let mut selected: usize = 0;

    render(stdout, title, options, selected)?;

    loop {
        if let Event::Key(key) = event::read()? {
            match key.code {
                KeyCode::Up | KeyCode::Char('k') => {
                    selected = selected.saturating_sub(1);
                }
                KeyCode::Down | KeyCode::Char('j') => {
                    selected = (selected + 1).min(options.len() - 1);
                }
                KeyCode::Char(c) if c.is_ascii_digit() => {
                    let n = c.to_digit(10).unwrap() as usize;
                    if n >= 1 && n <= options.len() {
                        selected = n - 1;
                    }
                }
                KeyCode::Enter => {
                    clear(stdout, title, options)?;
                    // Print the selection on one line
                    write!(
                        stdout,
                        "{} {}\r\n",
                        style::style(format!("{title}:")).bold(),
                        options[selected]
                    )?;
                    stdout.flush()?;
                    return Ok(Some(selected));
                }
                KeyCode::Esc => {
                    clear(stdout, title, options)?;
                    return Ok(None);
                }
                KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                    clear(stdout, title, options)?;
                    return Ok(None);
                }
                _ => {}
            }

            render(stdout, title, options, selected)?;
        }
    }
}

fn render(
    stdout: &mut io::Stdout,
    title: &str,
    options: &[String],
    selected: usize,
) -> anyhow::Result<()> {
    // Move cursor to start and clear
    clear(stdout, title, options)?;

    // Title
    write!(stdout, "{}\r\n", style::style(title).bold())?;

    for (i, option) in options.iter().enumerate() {
        let num = i + 1;
        if i == selected {
            write!(
                stdout,
                "  {} {}\r\n",
                style::style(format!("{num}")).cyan().bold(),
                style::style(option).bold()
            )?;
        } else {
            write!(
                stdout,
                "  {} {}\r\n",
                style::style(format!("{num}")).dark_grey(),
                option
            )?;
        }
    }

    stdout.flush()?;
    Ok(())
}

fn clear(stdout: &mut io::Stdout, _title: &str, options: &[String]) -> anyhow::Result<()> {
    // Move up to title line + all option lines, clear each
    let total_lines = 1 + options.len();
    for _ in 0..total_lines {
        stdout.execute(cursor::MoveUp(1))?;
        stdout.execute(terminal::Clear(terminal::ClearType::CurrentLine))?;
    }
    // One extra move down to stay at the right position isn't needed since we write from top
    Ok(())
}
