use super::*;
use pulldown_cmark::{CodeBlockKind, Event, Options, Parser, Tag, TagEnd};
#[derive(Default)]
struct Table {
    rows: Vec<(bool, Vec<Line<'static>>)>,
    row: Vec<Line<'static>>,
    cell: Vec<Span<'static>>,
}
struct Document {
    lines: Vec<Line<'static>>,
    line: Vec<Span<'static>>,
    strong: usize,
    emphasis: usize,
    strike: usize,
    heading: bool,
    code: bool,
    quotes: usize,
    lists: Vec<Option<u64>>,
    links: Vec<String>,
    table: Option<Table>,
    width: usize,
}
impl Document {
    fn flush(&mut self) {
        if !self.line.is_empty() {
            self.lines.push(Line::from(std::mem::take(&mut self.line)));
        }
    }
    fn blank(&mut self) {
        self.flush();
        if self.lines.last().is_some_and(|line| !line.spans.is_empty()) {
            self.lines.push(Line::raw(""));
        }
    }
    fn current_style(&self) -> Style {
        let mut result = style(if self.heading { BLUE } else { TEXT });
        if self.strong > 0 || self.heading {
            result = result.add_modifier(Modifier::BOLD);
        }
        if self.emphasis > 0 {
            result = result.add_modifier(Modifier::ITALIC);
        }
        if self.strike > 0 {
            result = result.add_modifier(Modifier::CROSSED_OUT);
        }
        if !self.links.is_empty() {
            result = result.fg(BLUE).add_modifier(Modifier::UNDERLINED);
        }
        result
    }
    fn push(&mut self, text: impl Into<String>, style: Style) {
        let span = Span::styled(text.into(), style);
        if let Some(table) = self.table.as_mut() {
            table.cell.push(span);
            return;
        }
        if self.line.is_empty() && self.quotes > 0 {
            self.line.push(label("│ ".repeat(self.quotes), MUTED));
        }
        self.line.push(span);
    }
    fn table(&mut self, table: Table) {
        self.blank();
        let columns = table
            .rows
            .iter()
            .map(|(_, row)| row.len())
            .max()
            .unwrap_or(0);
        if columns == 0 {
            return;
        }
        let available = self.width.saturating_sub((columns - 1) * 3).max(columns);
        let maxima = (0..columns)
            .map(|index| {
                table
                    .rows
                    .iter()
                    .filter_map(|(_, row)| row.get(index))
                    .map(Line::width)
                    .max()
                    .unwrap_or(1)
                    .max(1)
            })
            .collect::<Vec<_>>();
        let mut widths = vec![1usize; columns];
        let mut remaining = available.saturating_sub(columns);
        while remaining > 0 {
            let mut changed = false;
            for index in 0..columns {
                if widths[index] < maxima[index] && remaining > 0 {
                    widths[index] += 1;
                    remaining -= 1;
                    changed = true;
                }
            }
            if !changed {
                break;
            }
        }
        for (header, row) in table.rows {
            let mut spans = Vec::new();
            for (index, width) in widths.iter().enumerate() {
                if index > 0 {
                    spans.push(label(" │ ", MUTED));
                }
                let cell = row.get(index).cloned().unwrap_or_default();
                spans.extend(fit_cell(cell, *width, header));
            }
            self.lines.push(Line::from(spans));
            if header {
                self.lines.push(Line::styled(
                    widths
                        .iter()
                        .map(|width| "─".repeat(*width))
                        .collect::<Vec<_>>()
                        .join("─┼─"),
                    style(MUTED),
                ));
            }
        }
        self.lines.push(Line::raw(""));
    }
}
fn fit_cell(cell: Line<'static>, width: usize, header: bool) -> Vec<Span<'static>> {
    let truncated = cell.width() > width;
    let marker = if width >= 3 { "..." } else { "." };
    let limit = width.saturating_sub(if truncated { marker.len() } else { 0 });
    let mut result = Vec::new();
    let mut used = 0;
    for span in cell.spans {
        let mut text = String::new();
        for character in span.content.chars() {
            let mut candidate = text.clone();
            candidate.push(character);
            if used + Line::raw(candidate.clone()).width() > limit {
                break;
            }
            text = candidate;
        }
        used += Line::raw(text.clone()).width();
        if !text.is_empty() {
            result.push(Span::styled(
                text,
                if header {
                    span.style.add_modifier(Modifier::BOLD)
                } else {
                    span.style
                },
            ));
        }
        if used >= limit {
            break;
        }
    }
    if truncated {
        result.push(label(marker, MUTED));
        used += marker.len();
    }
    if used < width {
        result.push(Span::raw(" ".repeat(width - used)));
    }
    result
}
fn html_text(raw: &str) -> String {
    let mut result = String::new();
    let mut remaining = raw;
    while let Some(start) = remaining.find("<!--") {
        result.push_str(&remaining[..start]);
        let Some(end) = remaining[start + 4..].find("-->") else {
            return strip_tags(&result);
        };
        remaining = &remaining[start + 4 + end + 3..];
    }
    result.push_str(remaining);
    strip_tags(&result)
}
fn strip_tags(raw: &str) -> String {
    let mut tag = false;
    raw.chars()
        .filter(|character| match character {
            '<' => {
                tag = true;
                false
            }
            '>' => {
                tag = false;
                false
            }
            _ => !tag,
        })
        .collect()
}
pub(super) fn render(markdown: &str, width: u16) -> Vec<Line<'static>> {
    let input = clean(markdown);
    let mut doc = Document {
        lines: vec![],
        line: vec![],
        strong: 0,
        emphasis: 0,
        strike: 0,
        heading: false,
        code: false,
        quotes: 0,
        lists: vec![],
        links: vec![],
        table: None,
        width: usize::from(width.max(1)),
    };
    let options = Options::ENABLE_TABLES
        | Options::ENABLE_TASKLISTS
        | Options::ENABLE_STRIKETHROUGH
        | Options::ENABLE_FOOTNOTES;
    for event in Parser::new_ext(&input, options) {
        match event {
            Event::Start(Tag::Heading { .. }) => {
                doc.blank();
                doc.heading = true;
            }
            Event::End(TagEnd::Heading(_)) => {
                doc.flush();
                doc.heading = false;
                doc.blank();
            }
            Event::Start(Tag::Paragraph) => {}
            Event::End(TagEnd::Paragraph) => doc.blank(),
            Event::Start(Tag::Strong) => doc.strong += 1,
            Event::End(TagEnd::Strong) => doc.strong = doc.strong.saturating_sub(1),
            Event::Start(Tag::Emphasis) => doc.emphasis += 1,
            Event::End(TagEnd::Emphasis) => doc.emphasis = doc.emphasis.saturating_sub(1),
            Event::Start(Tag::Strikethrough) => doc.strike += 1,
            Event::End(TagEnd::Strikethrough) => doc.strike = doc.strike.saturating_sub(1),
            Event::Start(Tag::BlockQuote(_)) => {
                doc.flush();
                doc.quotes += 1;
            }
            Event::End(TagEnd::BlockQuote(_)) => {
                doc.flush();
                doc.quotes = doc.quotes.saturating_sub(1);
                doc.blank();
            }
            Event::Start(Tag::List(start)) => {
                doc.flush();
                doc.lists.push(start);
            }
            Event::End(TagEnd::List(_)) => {
                doc.flush();
                doc.lists.pop();
                if doc.lists.is_empty() {
                    doc.blank();
                }
            }
            Event::Start(Tag::Item) => {
                doc.flush();
                let indent = "  ".repeat(doc.lists.len().saturating_sub(1));
                let prefix = if let Some(Some(number)) = doc.lists.last_mut() {
                    let prefix = format!("{number}. ");
                    *number = number.saturating_add(1);
                    prefix
                } else {
                    "• ".into()
                };
                doc.push(format!("{indent}{prefix}"), style(MUTED));
            }
            Event::End(TagEnd::Item) => doc.flush(),
            Event::TaskListMarker(done) => {
                if let Some(prefix) = doc.line.last_mut() {
                    prefix.content = prefix.content.replace("• ", "").into();
                }
                doc.push(
                    if done { "[x] " } else { "[ ] " },
                    style(if done { GREEN } else { AMBER }),
                );
            }
            Event::Start(Tag::CodeBlock(kind)) => {
                doc.blank();
                if let CodeBlockKind::Fenced(language) = kind
                    && !language.is_empty()
                {
                    doc.lines
                        .push(Line::from(label(language.into_string(), MUTED)));
                }
                doc.code = true;
            }
            Event::End(TagEnd::CodeBlock) => {
                doc.flush();
                doc.code = false;
                doc.blank();
            }
            Event::Text(text) if doc.code => {
                let mut pieces = text.split('\n').peekable();
                while let Some(piece) = pieces.next() {
                    if !piece.is_empty() || pieces.peek().is_some() {
                        if doc.line.is_empty() {
                            doc.push("│ ", style(MUTED));
                        }
                        doc.push(piece.to_owned(), style(Color::Cyan));
                    }
                    if pieces.peek().is_some() {
                        doc.flush();
                    }
                }
            }
            Event::Text(text) => doc.push(text.into_string(), doc.current_style()),
            Event::Code(text) => doc.push(
                text.into_string(),
                style(Color::Cyan).add_modifier(Modifier::BOLD),
            ),
            Event::Start(Tag::Link { dest_url, .. }) => doc.links.push(dest_url.into_string()),
            Event::Start(Tag::Image { dest_url, .. }) => {
                doc.push("Image: ", style(MUTED));
                doc.links.push(dest_url.into_string());
            }
            Event::End(TagEnd::Link | TagEnd::Image) => {
                if let Some(url) = doc.links.pop() {
                    let text = doc
                        .line
                        .iter()
                        .map(|span| span.content.as_ref())
                        .collect::<String>();
                    if !url.is_empty() && !text.ends_with(&url) {
                        doc.push(format!(" ({url})"), style(MUTED));
                    }
                }
            }
            Event::SoftBreak => doc.push(" ", doc.current_style()),
            Event::HardBreak => doc.flush(),
            Event::Rule => {
                doc.blank();
                doc.lines
                    .push(Line::from(label("─".repeat(doc.width.min(60)), MUTED)));
                doc.blank();
            }
            Event::Start(Tag::Table(_)) => {
                doc.blank();
                doc.table = Some(Table::default());
            }
            Event::End(TagEnd::TableCell) => {
                if let Some(table) = doc.table.as_mut() {
                    table.row.push(Line::from(std::mem::take(&mut table.cell)));
                }
            }
            Event::End(TagEnd::TableHead | TagEnd::TableRow) => {
                if let Some(table) = doc.table.as_mut() {
                    table.rows.push((
                        matches!(event, Event::End(TagEnd::TableHead)),
                        std::mem::take(&mut table.row),
                    ));
                }
            }
            Event::End(TagEnd::Table) => {
                if let Some(table) = doc.table.take() {
                    doc.table(table);
                }
            }
            Event::Html(html) | Event::InlineHtml(html) => {
                if html.trim().eq_ignore_ascii_case("<br>")
                    || html.trim().eq_ignore_ascii_case("<br />")
                {
                    doc.flush();
                } else {
                    let text = html_text(&html);
                    if !text.trim().is_empty() {
                        doc.push(text, doc.current_style());
                    }
                }
            }
            Event::FootnoteReference(name) => doc.push(format!("[{name}]"), style(BLUE)),
            Event::Start(Tag::FootnoteDefinition(name)) => {
                doc.blank();
                doc.push(format!("[{name}] "), style(BLUE));
            }
            Event::End(TagEnd::FootnoteDefinition) => doc.blank(),
            _ => {}
        }
    }
    doc.flush();
    while doc.lines.last().is_some_and(|line| line.spans.is_empty()) {
        doc.lines.pop();
    }
    doc.lines
}
#[cfg(test)]
mod tests {
    use super::*;
    fn plain(lines: &[Line<'_>]) -> String {
        lines
            .iter()
            .map(|line| {
                line.spans
                    .iter()
                    .map(|span| span.content.as_ref())
                    .collect::<String>()
            })
            .collect::<Vec<_>>()
            .join("\n")
    }
    #[test]
    fn renders_structure_instead_of_markdown_syntax() {
        let rendered = render(
            "# Heading\n\n**bold** and *italic*, `code`\n\n- [ ] Work\n- [x] Done\n\n> quote\n\n[Guide](https://example.invalid)\n\n```rust\nlet x = 1;\n```\n<!-- hidden metadata -->",
            80,
        );
        let text = plain(&rendered);
        assert!(
            text.contains("Heading")
                && text.contains("[ ] Work")
                && text.contains("[x] Done")
                && text.contains("│ quote")
                && text.contains("Guide (https://example.invalid)")
                && text.contains("let x = 1;")
        );
        assert!(!text.contains("**") && !text.contains("```") && !text.contains("hidden metadata"));
        assert!(
            rendered
                .iter()
                .flat_map(|line| &line.spans)
                .any(|span| span.content == "bold"
                    && span.style.add_modifier.contains(Modifier::BOLD))
        );
        assert!(
            rendered
                .iter()
                .flat_map(|line| &line.spans)
                .all(|span| span.style.bg.is_none())
        );
    }
    #[test]
    fn tables_fit_the_viewport_and_keep_headers() {
        let rendered = render(
            "| Project | Status |\n|---|---|\n| A very long project name | Ready |\n| 短い | Done |",
            24,
        );
        assert!(rendered.iter().all(|line| line.width() <= 24));
        assert!(plain(&rendered).contains("Status"));
        assert!(plain(&rendered).contains('│'));
    }
}
