use owo_colors::Style;
use std::io::IsTerminal;

const EOL: &str = "\r\n";

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ColorPreference {
    #[default]
    Auto,
    Always,
    Never,
}

impl ColorPreference {
    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "auto" => Some(Self::Auto),
            "always" => Some(Self::Always),
            "never" => Some(Self::Never),
            _ => None,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ColorContext {
    enabled: bool,
}

impl ColorContext {
    pub fn from_preference(preference: ColorPreference) -> Self {
        let enabled = Self::resolve_enabled(
            preference,
            std::io::stdout().is_terminal(),
            std::env::var_os("NO_COLOR").is_some(),
        );
        Self { enabled }
    }

    pub fn colorize_output(&self, text: &str) -> String {
        if !self.enabled || text.is_empty() {
            return text.to_owned();
        }

        text.split_inclusive('\n')
            .map(|line| {
                let (content, ending) = split_line_ending(line);
                format!("{}{}", self.colorize_line(content), ending)
            })
            .collect()
    }

    pub fn error_prefix(&self, text: &str) -> String {
        self.paint(text, Style::new().red().bold())
    }

    pub fn warn_prefix(&self, text: &str) -> String {
        self.paint(text, Style::new().yellow().bold())
    }

    pub fn info_prefix(&self, text: &str) -> String {
        self.paint(text, Style::new().bold().dimmed())
    }

    pub fn success_text(&self, text: &str) -> String {
        self.paint(text, Style::new().green().bold())
    }

    pub fn emphasis(&self, text: &str) -> String {
        self.paint(text, Style::new().cyan())
    }

    pub fn value_added(&self, text: &str) -> String {
        self.paint(text, Style::new().green())
    }

    pub fn value_removed(&self, text: &str) -> String {
        self.paint(text, Style::new().yellow())
    }

    fn colorize_line(&self, line: &str) -> String {
        if let Some(rest) = line.strip_prefix("ERROR ") {
            return format!(
                "{} {}",
                self.error_prefix("ERROR"),
                self.colorize_prefixed_rest(rest)
            );
        }
        if let Some(rest) = line.strip_prefix("WARN  ") {
            return format!(
                "{}  {}",
                self.warn_prefix("WARN"),
                self.colorize_prefixed_rest(rest)
            );
        }
        if let Some(rest) = line.strip_prefix("INFO  ") {
            return format!(
                "{}  {}",
                self.info_prefix("INFO"),
                self.colorize_prefixed_rest(rest)
            );
        }
        self.colorize_unprefixed(line)
    }

    fn colorize_prefixed_rest(&self, rest: &str) -> String {
        let with_label = self.colorize_label_before_colon(rest);
        let with_quotes = self.colorize_quoted_segments(&with_label);
        self.colorize_semantic_values(&with_quotes)
    }

    fn colorize_unprefixed(&self, line: &str) -> String {
        if line == "Global options:" {
            return self.emphasis(line);
        }
        if let Some(rest) = line.strip_prefix("  --color <auto|always|never>  ") {
            return format!(
                "  {}  {}",
                self.emphasis("--color <auto|always|never>"),
                rest
            );
        }
        if is_success_line(line) {
            return self.success_text(line);
        }
        if let Some(rendered) = self.colorize_shim_alter_line(line) {
            return rendered;
        }
        if line.starts_with('\'') {
            let with_quotes = self.colorize_quoted_segments(line);
            return self.colorize_semantic_values(&with_quotes);
        }
        line.to_owned()
    }

    fn colorize_label_before_colon(&self, line: &str) -> String {
        let Some((label, rest)) = line.split_once(':') else {
            return line.to_owned();
        };
        if label.is_empty() || label.contains(' ') {
            return line.to_owned();
        }
        format!("{}:{}", self.emphasis(label), rest)
    }

    fn colorize_quoted_segments(&self, line: &str) -> String {
        let mut output = String::new();
        let mut remainder = line;
        loop {
            let Some(start) = remainder.find('\'') else {
                output.push_str(remainder);
                break;
            };
            let after_start = &remainder[start + 1..];
            let Some(end) = after_start.find('\'') else {
                output.push_str(remainder);
                break;
            };
            output.push_str(&remainder[..start]);
            output.push('\'');
            output.push_str(&self.emphasis(&after_start[..end]));
            output.push('\'');
            remainder = &after_start[end + 1..];
        }
        output
    }

    fn colorize_semantic_values(&self, line: &str) -> String {
        if let Some(rendered) = self.colorize_arrow_delta(line) {
            return rendered;
        }
        if let Some(rendered) = self.colorize_from_to_delta(line) {
            return rendered;
        }
        line.to_owned()
    }

    fn colorize_shim_alter_line(&self, line: &str) -> Option<String> {
        let marker = " is now using ";
        let instead = " instead of ";
        let name_end = line.find(marker)?;
        let source_end = line[name_end + marker.len()..].find(instead)?;
        let source_end = name_end + marker.len() + source_end;
        let from_end = line[source_end + instead.len()..].find('.')?;
        let from_end = source_end + instead.len() + from_end;
        Some(format!(
            "{}{}{}{}{}.",
            self.emphasis(&line[..name_end]),
            marker,
            self.value_added(&line[name_end + marker.len()..source_end]),
            instead,
            self.value_removed(&line[source_end + instead.len()..from_end])
        ))
    }

    fn colorize_arrow_delta(&self, line: &str) -> Option<String> {
        let (before, after_colon) = line.split_once(": ")?;
        let arrow_index = after_colon.find(" -> ")?;
        let old = &after_colon[..arrow_index];
        let new = &after_colon[arrow_index + 4..];
        if old.is_empty() || new.is_empty() {
            return None;
        }
        Some(format!(
            "{}: {} -> {}",
            before,
            self.value_removed(old),
            self.value_added(new)
        ))
    }

    fn colorize_from_to_delta(&self, line: &str) -> Option<String> {
        let from_index = line.find(" from ")?;
        let to_index = line[from_index + 6..].find(" to ")?;
        let to_index = from_index + 6 + to_index;
        let old = &line[from_index + 6..to_index];
        let after_to = &line[to_index + 4..];
        let new_end = after_to.find('.')?;
        let new = &after_to[..new_end];
        if old.is_empty() || new.is_empty() {
            return None;
        }
        Some(format!(
            "{}{} to {}{}",
            &line[..from_index + 6],
            self.value_removed(old),
            self.value_added(new),
            &after_to[new_end..]
        ))
    }

    fn paint(&self, text: &str, style: Style) -> String {
        if self.enabled {
            style.style(text).to_string()
        } else {
            text.to_owned()
        }
    }

    fn resolve_enabled(
        preference: ColorPreference,
        stdout_is_terminal: bool,
        no_color_present: bool,
    ) -> bool {
        match preference {
            ColorPreference::Always => true,
            ColorPreference::Never => false,
            ColorPreference::Auto => stdout_is_terminal && !no_color_present,
        }
    }
}

fn split_line_ending(line: &str) -> (&str, &str) {
    if let Some(content) = line.strip_suffix(EOL) {
        return (content, EOL);
    }
    if let Some(content) = line.strip_suffix('\n') {
        return (content, "\n");
    }
    (line, "")
}

fn is_success_line(line: &str) -> bool {
    matches!(
        line,
        "Scoop was updated successfully!"
            | "Scoop is up to date."
            | "Everything is ok!"
            | "Everything is shiny now!"
    ) || line.contains(" was installed successfully!")
        || line.contains(" was downloaded successfully!")
        || line.contains(" bucket was added successfully.")
        || line.contains(" bucket was removed successfully.")
}

#[cfg(test)]
mod tests {
    use super::{ColorContext, ColorPreference};
    use regex::Regex;

    #[test]
    fn parses_color_preference_variants() {
        assert_eq!(ColorPreference::parse("auto"), Some(ColorPreference::Auto));
        assert_eq!(
            ColorPreference::parse("always"),
            Some(ColorPreference::Always)
        );
        assert_eq!(
            ColorPreference::parse("never"),
            Some(ColorPreference::Never)
        );
        assert_eq!(ColorPreference::parse("bad"), None);
    }

    #[test]
    fn always_enables_color() {
        assert!(ColorContext::resolve_enabled(
            ColorPreference::Always,
            false,
            true
        ));
    }

    #[test]
    fn never_disables_color() {
        assert!(!ColorContext::resolve_enabled(
            ColorPreference::Never,
            true,
            false
        ));
    }

    #[test]
    fn auto_respects_tty_and_no_color() {
        assert!(ColorContext::resolve_enabled(
            ColorPreference::Auto,
            true,
            false
        ));
        assert!(!ColorContext::resolve_enabled(
            ColorPreference::Auto,
            false,
            false
        ));
        assert!(!ColorContext::resolve_enabled(
            ColorPreference::Auto,
            true,
            true
        ));
    }

    #[test]
    fn colorizes_prefixed_messages_and_success_lines() {
        let colors = ColorContext::from_preference(ColorPreference::Always);
        let rendered = colors.colorize_output(
            "WARN  scoop: 'missing' isn't a scoop command. See 'scoop help'.\r\nScoop is up to date.\r\n",
        );

        assert!(rendered.contains("\u{1b}["));
        assert!(rendered.contains("Scoop is up to date."));
    }

    #[test]
    fn colorizes_shim_alter_and_version_delta_lines() {
        let colors = ColorContext::from_preference(ColorPreference::Always);
        let rendered = colors.colorize_output(
            "demo is now using other instead of main.\r\n'demo' was updated from 1.0.0 to 1.1.0.\r\n",
        );

        assert!(rendered.contains("\u{1b}["));
        let plain = strip_ansi(&rendered);
        assert!(plain.contains("demo is now using other instead of main."));
        assert!(plain.contains("1.0.0"));
        assert!(plain.contains("1.1.0"));
    }

    fn strip_ansi(text: &str) -> String {
        Regex::new(r"\x1B\[[0-9;]*m")
            .expect("ansi regex should compile")
            .replace_all(text, "")
            .into_owned()
    }
}
