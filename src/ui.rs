use ratatui::{
    Frame,
    layout::{Alignment, Constraint, Direction, Layout, Margin, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Cell, Clear, List, ListItem, Paragraph, Row, Table, Wrap},
};
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

use crate::app::{App, EnvEntry, EnvSource};

pub fn draw(frame: &mut Frame<'_>, app: &App) {
    let registry_mode = app
        .entries
        .iter()
        .any(|entry| matches!(entry.source, EnvSource::User | EnvSource::System));
    let areas = if registry_mode {
        Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Min(3),
                Constraint::Min(3),
                Constraint::Length(1),
            ])
            .split(frame.area())
    } else {
        Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Min(3), Constraint::Length(1)])
            .split(frame.area())
    };

    if registry_mode {
        render_table(
            frame,
            areas[0],
            app,
            Some(EnvSource::User),
            " User Variables ",
        );
        render_table(
            frame,
            areas[1],
            app,
            Some(EnvSource::System),
            " System Variables ",
        );
    } else {
        let title = app
            .entries
            .first()
            .map(|entry| format!(" {} Variables ", entry.source.label()))
            .unwrap_or_else(|| " Environment Variables ".to_owned());
        render_table(
            frame,
            areas[0],
            app,
            app.entries.first().map(|e| e.source),
            &title,
        );
    }

    let dirty_marker = if app.dirty { " [UNSAVED]" } else { "" };
    let help = Line::from(format!("{}{}", app.status, dirty_marker));
    let status_area = areas[areas.len() - 1];
    frame.render_widget(help, status_area.inner(Margin::new(1, 0)));

    if let Some(draft) = app.add_variable.as_ref() {
        let popup = centered_rect(65, 30, frame.area());
        frame.render_widget(Clear, popup);
        let key_marker = if draft.editing_value { "  " } else { "▶ " };
        let value_marker = if draft.editing_value { "▶ " } else { "  " };
        let lines = vec![
            Line::from(vec![
                Span::raw(key_marker),
                Span::styled("KEY  ", Style::default().fg(Color::Cyan)),
                Span::raw(" : "),
                Span::raw(draft.key.as_str()),
            ]),
            Line::from(vec![
                Span::raw(value_marker),
                Span::styled("VALUE", Style::default().fg(Color::Cyan)),
                Span::raw(" : "),
                Span::raw(draft.value.as_str()),
            ]),
        ];
        let popup_widget = Paragraph::new(lines).block(
            Block::default()
                .title(format!(
                    " Add variable to {} — Tab: switch, Enter: add, Esc: cancel ",
                    draft.source.label()
                ))
                .borders(Borders::ALL),
        );
        frame.render_widget(popup_widget, popup);
        let prefix_width = if draft.editing_value {
            10 + draft.value.width()
        } else {
            10 + draft.key.width()
        };
        let cursor_x =
            popup.x + 1 + prefix_width.min(usize::from(popup.width.saturating_sub(2))) as u16;
        let cursor_y = popup.y + if draft.editing_value { 2 } else { 1 };
        frame.set_cursor_position((cursor_x, cursor_y));
    } else if let Some(index) = app.delete_confirmation {
        let key = app
            .entries
            .get(index)
            .map(|entry| entry.key.as_str())
            .unwrap_or("selected variable");
        let popup = centered_rect(60, 20, frame.area());
        frame.render_widget(Clear, popup);
        let prompt = Paragraph::new(format!("Delete {key}?\n\nY/Enter: delete    N/Esc: cancel"))
            .alignment(Alignment::Center)
            .block(
                Block::default()
                    .title(" Confirm deletion ")
                    .borders(Borders::ALL),
            );
        frame.render_widget(prompt, popup);
    } else if app.edit_as_list {
        let popup = centered_rect(85, 75, frame.area());
        frame.render_widget(Clear, popup);
        let variable_name = app
            .entries
            .get(app.selected)
            .map(|entry| entry.key.as_str())
            .unwrap_or("Environment variable");
        let title = format!(
            " {variable_name} entries — ↑/↓/PgUp/PgDn select, Enter edit, a add, d delete, Ctrl+↑/↓ move, Ctrl+Enter apply "
        );
        let items = app
            .list_items
            .as_deref()
            .unwrap_or_default()
            .iter()
            .map(|item| ListItem::new(item.as_str()));
        let list = List::new(items)
            .highlight_style(
                Style::default()
                    .bg(Color::DarkGray)
                    .add_modifier(Modifier::BOLD),
            )
            .highlight_symbol("▶ ")
            .block(Block::default().title(title).borders(Borders::ALL));
        let mut state = ratatui::widgets::ListState::default();
        if app
            .list_items
            .as_ref()
            .is_some_and(|items| !items.is_empty())
        {
            state.select(Some(app.list_selected));
        }
        frame.render_stateful_widget(list, popup, &mut state);

        if app.editing_list_item {
            if let Some(value) = app.edit_buffer.as_deref() {
                let input_popup = centered_rect(75, 20, frame.area());
                frame.render_widget(Clear, input_popup);
                let visible_width = usize::from(input_popup.width.saturating_sub(2)).max(1);
                let prefix: String = value.chars().take(app.edit_cursor).collect();
                let cursor_column = prefix.width();
                let horizontal_offset =
                    cursor_column.saturating_sub(visible_width.saturating_sub(1));
                let input = Paragraph::new(value)
                    .block(
                        Block::default()
                            .title(" Edit one entry — Enter apply, Esc cancel ")
                            .borders(Borders::ALL),
                    )
                    .scroll((0, horizontal_offset.min(usize::from(u16::MAX)) as u16));
                frame.render_widget(input, input_popup);
                let cursor_x = input_popup.x
                    + 1
                    + cursor_column
                        .saturating_sub(horizontal_offset)
                        .min(visible_width.saturating_sub(1)) as u16;
                frame.set_cursor_position((cursor_x, input_popup.y + 1));
            }
        }
    } else if let Some(value) = app.edit_buffer.as_deref() {
        let popup = centered_rect(80, 60, frame.area());
        frame.render_widget(Clear, popup);
        let content_width = usize::from(popup.width.saturating_sub(2)).max(1);
        let content_height = usize::from(popup.height.saturating_sub(2)).max(1);
        let (cursor_row, cursor_column) = app.edit_cursor_position(content_width);
        let vertical_offset = cursor_row.saturating_sub(content_height.saturating_sub(1));
        let title = if app.edit_as_list {
            " Edit list — one entry per line; Shift+Enter: new entry "
        } else {
            " Edit value — Enter: apply, Shift+Enter: new line "
        };
        let input = Paragraph::new(value)
            .block(Block::default().title(title).borders(Borders::ALL))
            .alignment(Alignment::Left)
            .wrap(Wrap { trim: false })
            .scroll((vertical_offset.min(usize::from(u16::MAX)) as u16, 0));
        frame.render_widget(input, popup);
        let cursor_x = popup.x + 1 + cursor_column.min(content_width.saturating_sub(1)) as u16;
        let cursor_y = popup.y + 1 + cursor_row.saturating_sub(vertical_offset) as u16;
        frame.set_cursor_position((cursor_x, cursor_y));
    }
}

fn render_table(
    frame: &mut Frame<'_>,
    area: Rect,
    app: &App,
    source: Option<EnvSource>,
    title: &str,
) {
    let entries: Vec<(usize, &EnvEntry)> = app
        .entries
        .iter()
        .enumerate()
        .filter(|(_, entry)| source.is_none_or(|source| entry.source == source))
        .collect();
    let mut rows: Vec<(Option<usize>, Row<'_>)> = Vec::new();
    let comment_width = usize::from(area.width).saturating_sub(8).max(1);
    for (global_index, entry) in entries {
        for line in &entry.leading_lines {
            append_comment_rows(&mut rows, line, comment_width);
        }
        rows.push((
            Some(global_index),
            Row::new([
                Cell::from(entry.key.as_str()),
                Cell::from(entry.value.as_str()),
            ]),
        ));
        for line in &entry.trailing_lines {
            append_comment_rows(&mut rows, line, comment_width);
        }
    }
    let selected_in_group = rows
        .iter()
        .position(|(global_index, _)| *global_index == Some(app.selected));
    let group_active = selected_in_group.is_some();
    let table_rows = rows.iter().map(|(_, row)| row.clone());
    let table = Table::new(
        table_rows,
        [Constraint::Percentage(30), Constraint::Percentage(70)],
    )
    .header(
        Row::new([Cell::from("KEY"), Cell::from("VALUE")]).style(
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        ),
    )
    .row_highlight_style(if group_active {
        Style::default()
            .bg(Color::DarkGray)
            .add_modifier(Modifier::BOLD)
    } else {
        Style::default()
    })
    .highlight_symbol(if group_active { "▶ " } else { "  " })
    .block(Block::default().title(title).borders(Borders::ALL));

    let mut state = ratatui::widgets::TableState::default();
    state.select(selected_in_group.or_else(|| (!rows.is_empty()).then_some(0)));
    frame.render_stateful_widget(table, area, &mut state);
}

fn append_comment_rows<'a>(rows: &mut Vec<(Option<usize>, Row<'a>)>, line: &str, width: usize) {
    if line.is_empty() {
        rows.push((None, Row::new([Cell::from(""), Cell::from("")])));
        return;
    }

    let mut wrapped = Vec::new();
    let mut chunk = String::new();
    let mut chunk_width = 0;
    for character in line.chars() {
        let character_width = UnicodeWidthChar::width(character).unwrap_or(0);
        if chunk_width > 0 && chunk_width + character_width > width {
            wrapped.push(std::mem::take(&mut chunk));
            chunk_width = 0;
        }
        chunk.push(character);
        chunk_width += character_width;
    }
    if !chunk.is_empty() {
        wrapped.push(chunk);
    }

    for (index, text) in wrapped.into_iter().enumerate() {
        let text = if index == 0 {
            text
        } else {
            format!("  {text}")
        };
        rows.push((
            None,
            Row::new([Cell::from(text).column_span(2)]).style(Style::default().fg(Color::Gray)),
        ));
    }
}

fn centered_rect(width_percent: u16, height_percent: u16, area: Rect) -> Rect {
    let vertical = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Percentage((100 - height_percent) / 2),
            Constraint::Percentage(height_percent),
            Constraint::Percentage((100 - height_percent) / 2),
        ])
        .split(area);
    Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage((100 - width_percent) / 2),
            Constraint::Percentage(width_percent),
            Constraint::Percentage((100 - width_percent) / 2),
        ])
        .split(vertical[1])[1]
}
