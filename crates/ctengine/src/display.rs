use nu_ansi_term::{Color, Style};
use std::io::{self, IsTerminal, Write};
use std::sync::OnceLock;
use terminal_size::{Height, Width, terminal_size};
use unicode_width::UnicodeWidthChar;

const TABLE_TAB_WIDTH: usize = 8;

#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum ColAlign {
    Left,
    Right,
}

pub(crate) fn render_ascii_table(
    headers: Vec<String>,
    rows: Vec<Vec<String>>,
    aligns: Vec<ColAlign>,
) -> String {
    let mut widths: Vec<usize> = headers.iter().map(|h| stripped_width(h)).collect();
    for row in &rows {
        for (idx, cell) in row.iter().enumerate() {
            if idx >= widths.len() {
                widths.push(0);
            }
            widths[idx] = widths[idx].max(stripped_width(cell));
        }
    }

    fit_widths_to_terminal(&mut widths);

    let mut out = String::new();
    out.push_str(&render_table_row(&headers, &widths, &aligns, true));
    out.push('\n');

    let mut sep = String::new();
    sep.push('+');
    for width in &widths {
        sep.push_str(&"-".repeat(*width + 2));
        sep.push('+');
    }
    out.push_str(&sep);

    for row in rows {
        out.push('\n');
        out.push_str(&render_table_row(&row, &widths, &aligns, false));
    }
    out
}

fn render_table_row(
    cells: &[String],
    widths: &[usize],
    aligns: &[ColAlign],
    header_row: bool,
) -> String {
    let mut s = String::new();
    let use_color = use_color_output();
    s.push('|');
    for (idx, width) in widths.iter().enumerate() {
        let cell = cells.get(idx).map(String::as_str).unwrap_or("");
        let normalized = normalize_table_cell(cell);
        let expanded = expand_tabs(&normalized, TABLE_TAB_WIDTH);
        let mut clipped = clip_with_ellipsis(&expanded, *width);
        if use_color && header_row {
            clipped = Style::new()
                .fg(Color::Cyan)
                .bold()
                .paint(clipped)
                .to_string();
        }
        let cell_len = stripped_width(&clipped);
        let padding = width.saturating_sub(cell_len);
        s.push(' ');
        if aligns.get(idx) == Some(&ColAlign::Right) {
            s.push_str(&" ".repeat(padding));
            s.push_str(&clipped);
            s.push(' ');
        } else {
            s.push_str(&clipped);
            s.push_str(&" ".repeat(padding + 1));
        }
        s.push('|');
    }
    s
}

fn normalize_table_cell(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut pending_space = false;
    for ch in s.chars() {
        match ch {
            '\r' | '\n' => {
                pending_space = !out.is_empty();
            }
            _ => {
                if pending_space && !matches!(ch, ' ' | '\t') {
                    out.push(' ');
                }
                pending_space = false;
                out.push(ch);
            }
        }
    }
    out
}

pub(crate) fn stripped_width(s: &str) -> usize {
    let mut count = 0usize;
    let mut in_escape = false;
    for ch in s.chars() {
        if in_escape {
            if ch == 'm' {
                in_escape = false;
            }
            continue;
        }
        if ch == '\u{1b}' {
            in_escape = true;
            continue;
        }
        if ch == '\t' {
            let next_tab_stop = ((count / TABLE_TAB_WIDTH) + 1) * TABLE_TAB_WIDTH;
            count = next_tab_stop;
            continue;
        }
        count += UnicodeWidthChar::width(ch).unwrap_or(0);
    }
    count
}

fn expand_tabs(s: &str, tab_width: usize) -> String {
    let mut out = String::with_capacity(s.len());
    let mut col = 0usize;
    let mut in_escape = false;
    for ch in s.chars() {
        if in_escape {
            out.push(ch);
            if ch == 'm' {
                in_escape = false;
            }
            continue;
        }
        if ch == '\u{1b}' {
            in_escape = true;
            out.push(ch);
            continue;
        }
        if ch == '\t' {
            let next_tab_stop = ((col / tab_width) + 1) * tab_width;
            let spaces = next_tab_stop.saturating_sub(col);
            out.push_str(&" ".repeat(spaces));
            col = next_tab_stop;
            continue;
        }
        out.push(ch);
        col += UnicodeWidthChar::width(ch).unwrap_or(0);
    }
    out
}

pub(crate) fn clip_with_ellipsis(s: &str, width: usize) -> String {
    if stripped_width(s) <= width {
        return s.to_string();
    }
    if width <= 3 {
        return ".".repeat(width);
    }
    let mut out = String::new();
    let max_content_width = width - 3;
    let mut used_width = 0usize;
    for ch in s.chars() {
        let ch_width = UnicodeWidthChar::width(ch).unwrap_or(0);
        if used_width + ch_width > max_content_width {
            break;
        }
        out.push(ch);
        used_width += ch_width;
    }
    out.push_str("...");
    out
}

fn fit_widths_to_terminal(widths: &mut [usize]) {
    let Some((Width(w), _)) = terminal_size() else {
        return;
    };
    if widths.is_empty() {
        return;
    }
    let border = 1 + widths.len() * 3;
    let mut total = border + widths.iter().sum::<usize>();
    let max_total = w as usize;
    if total <= max_total {
        return;
    }

    let min_col = 6usize;
    while total > max_total {
        let mut shrunk = false;
        for width in widths.iter_mut() {
            if *width > min_col && total > max_total {
                *width -= 1;
                total -= 1;
                shrunk = true;
            }
        }
        if !shrunk {
            break;
        }
    }
}

fn use_color_output() -> bool {
    static USE_COLOR_OUTPUT_CACHE: OnceLock<bool> = OnceLock::new();
    *USE_COLOR_OUTPUT_CACHE.get_or_init(resolve_use_color_output)
}

fn resolve_use_color_output() -> bool {
    match std::env::var("SYSKITS_REPL_COLOR")
        .ok()
        .unwrap_or_else(|| "auto".to_string())
        .to_ascii_lowercase()
        .as_str()
    {
        "always" | "on" | "true" => true,
        "never" | "off" | "false" => false,
        _ => io::stdout().is_terminal(),
    }
}

fn pager_enabled() -> bool {
    static PAGER_ENABLED_CACHE: OnceLock<bool> = OnceLock::new();
    *PAGER_ENABLED_CACHE.get_or_init(resolve_pager_enabled)
}

fn resolve_pager_enabled() -> bool {
    let interactive_tty = io::stdin().is_terminal() && io::stdout().is_terminal();
    match std::env::var("SYSKITS_REPL_PAGER")
        .ok()
        .unwrap_or_else(|| "auto".to_string())
        .to_ascii_lowercase()
        .as_str()
    {
        "off" | "never" | "false" | "0" => false,
        _ => interactive_tty,
    }
}

#[cfg(test)]
mod tests {
    use super::{ColAlign, expand_tabs, normalize_table_cell, render_ascii_table, stripped_width};

    #[test]
    fn stripped_width_counts_tabs_by_tab_stop() {
        assert_eq!(stripped_width("A\t1"), 9);
        assert_eq!(stripped_width("AB\t1"), 9);
    }

    #[test]
    fn expand_tabs_replaces_tabs_with_spaces() {
        assert_eq!(expand_tabs("A\t1", 8), "A       1");
        assert_eq!(expand_tabs("AB\t1", 8), "AB      1");
    }

    #[test]
    fn normalize_table_cell_flattens_newlines() {
        assert_eq!(normalize_table_cell("a\nb\n"), "a b");
        assert_eq!(normalize_table_cell("a\r\nb"), "a b");
    }

    #[test]
    fn render_ascii_table_aligns_cells_with_tabs() {
        let table = render_ascii_table(
            vec!["mode".into(), "row_index".into(), "line".into()],
            vec![vec!["parallel".into(), "1".into(), "A\t1".into()]],
            vec![ColAlign::Left, ColAlign::Right, ColAlign::Left],
        );

        let lines = table.lines().collect::<Vec<_>>();
        assert_eq!(lines.len(), 3);
        assert_eq!(stripped_width(lines[0]), stripped_width(lines[2]));
        assert!(!lines[2].contains('\t'));
    }

    #[test]
    fn render_ascii_table_flattens_multiline_cells() {
        let table = render_ascii_table(
            vec!["line_index".into(), "format_string".into()],
            vec![vec!["1".into(), "a\nb\n".into()]],
            vec![ColAlign::Right, ColAlign::Left],
        );

        let lines = table.lines().collect::<Vec<_>>();
        assert_eq!(lines.len(), 3);
        assert!(lines[2].contains("a b"));
        assert!(!lines[2].contains('\n'));
    }
}

pub(crate) fn print_block_with_pager(block: &str) {
    if block.is_empty() {
        return;
    }

    let lines: Vec<&str> = block.lines().collect();
    if lines.is_empty() {
        return;
    }
    if !pager_enabled() {
        println!("{block}");
        return;
    }

    let page_lines = terminal_size()
        .map(|(_, Height(h))| (h as usize).saturating_sub(2).max(1))
        .unwrap_or(24);

    if lines.len() <= page_lines {
        println!("{block}");
        return;
    }

    let mut idx = 0usize;
    while idx < lines.len() {
        let end = (idx + page_lines).min(lines.len());
        for line in &lines[idx..end] {
            println!("{line}");
        }
        idx = end;
        if idx >= lines.len() {
            break;
        }

        print!("--More-- (Enter: next, q: quit) ");
        let _ = io::stdout().flush();
        let mut answer = String::new();
        let _ = io::stdin().read_line(&mut answer);
        print!("\r\x1b[2K");
        let _ = io::stdout().flush();
        if answer.trim().eq_ignore_ascii_case("q") {
            break;
        }
    }
}
