use unicode_width::UnicodeWidthChar;

const DEFAULT_STATUS: &str =
    "↑/↓ or j/k: navigate   E/Enter: edit   A: add   d: delete   S: save   q: quit";
const SYSTEM_ACCESS_WARNING: &str = "Administrator rights required to change System variables";

#[derive(Debug, Clone)]
pub struct EnvEntry {
    pub key: String,
    pub value: String,
    pub source: EnvSource,
    pub expandable: bool,
    /// Dotenv comments and blank lines immediately preceding this assignment.
    pub leading_lines: Vec<String>,
    /// Dotenv comments and blank lines after the last assignment.
    pub trailing_lines: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EnvSource {
    Dotenv,
    User,
    System,
    #[allow(dead_code)]
    Process,
}

impl EnvSource {
    pub fn label(self) -> &'static str {
        match self {
            Self::Dotenv => ".env",
            Self::User => "User",
            Self::System => "System",
            Self::Process => "Process",
        }
    }
}

#[derive(Debug)]
pub struct App {
    pub entries: Vec<EnvEntry>,
    pub original_entries: Vec<EnvEntry>,
    pub selected: usize,
    pub edit_buffer: Option<String>,
    pub edit_cursor: usize,
    pub edit_as_list: bool,
    pub list_items: Option<Vec<String>>,
    pub list_selected: usize,
    pub editing_list_item: bool,
    pub add_variable: Option<NewVariableDraft>,
    pub delete_confirmation: Option<usize>,
    pub dirty: bool,
    pub status: String,
    pub system_write_allowed: bool,
}

#[derive(Debug)]
pub struct NewVariableDraft {
    pub key: String,
    pub value: String,
    pub editing_value: bool,
    pub source: EnvSource,
}

impl App {
    pub fn new(entries: Vec<EnvEntry>) -> Self {
        Self {
            original_entries: entries.clone(),
            entries,
            selected: 0,
            edit_buffer: None,
            edit_cursor: 0,
            edit_as_list: false,
            list_items: None,
            list_selected: 0,
            editing_list_item: false,
            add_variable: None,
            delete_confirmation: None,
            dirty: false,
            status: String::from(DEFAULT_STATUS),
            system_write_allowed: true,
        }
    }

    pub fn set_system_write_allowed(&mut self, allowed: bool) {
        self.system_write_allowed = allowed;
        if !allowed
            && self
                .entries
                .iter()
                .any(|entry| entry.source == EnvSource::System)
        {
            self.status = SYSTEM_ACCESS_WARNING.into();
        }
    }

    pub fn dismiss_system_access_warning(&mut self) {
        if self.status == SYSTEM_ACCESS_WARNING {
            self.status = DEFAULT_STATUS.into();
        }
    }

    pub fn show_system_access_warning_if_selected(&mut self) {
        if !self.system_write_allowed
            && self
                .entries
                .get(self.selected)
                .is_some_and(|entry| entry.source == EnvSource::System)
        {
            self.status = SYSTEM_ACCESS_WARNING.into();
        }
    }

    pub fn select_next(&mut self) {
        if !self.entries.is_empty() {
            self.selected = (self.selected + 1).min(self.entries.len() - 1);
        }
    }

    pub fn select_previous(&mut self) {
        if !self.entries.is_empty() {
            self.selected = self.selected.saturating_sub(1);
        }
    }

    pub fn begin_add_variable(&mut self) {
        let source = self.entries.get(self.selected).map_or_else(
            || {
                if cfg!(windows) {
                    EnvSource::User
                } else {
                    EnvSource::Dotenv
                }
            },
            |entry| entry.source,
        );
        if source == EnvSource::System && !self.system_write_allowed {
            self.status = SYSTEM_ACCESS_WARNING.into();
            return;
        }
        self.add_variable = Some(NewVariableDraft {
            key: String::new(),
            value: String::new(),
            editing_value: false,
            source,
        });
    }

    pub fn add_variable_char(&mut self, character: char) {
        if let Some(draft) = self.add_variable.as_mut() {
            if draft.editing_value {
                draft.value.push(character);
            } else {
                draft.key.push(character);
            }
        }
    }

    pub fn add_variable_backspace(&mut self) {
        if let Some(draft) = self.add_variable.as_mut() {
            if draft.editing_value {
                draft.value.pop();
            } else {
                draft.key.pop();
            }
        }
    }

    pub fn toggle_add_variable_field(&mut self) {
        if let Some(draft) = self.add_variable.as_mut() {
            draft.editing_value = !draft.editing_value;
        }
    }

    pub fn cancel_add_variable(&mut self) {
        self.add_variable = None;
    }

    pub fn commit_add_variable(&mut self) {
        let Some(draft) = self.add_variable.as_ref() else {
            return;
        };
        if draft.key.trim().is_empty() {
            self.status = "Variable key cannot be empty".into();
            return;
        }
        if self.entries.iter().any(|entry| {
            entry.source == draft.source && entry.key.eq_ignore_ascii_case(draft.key.trim())
        }) {
            self.status = format!("Variable {} already exists", draft.key.trim());
            return;
        }
        let Some(draft) = self.add_variable.take() else {
            return;
        };
        self.entries.push(EnvEntry {
            key: draft.key.trim().to_owned(),
            value: draft.value,
            source: draft.source,
            expandable: false,
            leading_lines: Vec::new(),
            trailing_lines: Vec::new(),
        });
        self.selected = self.entries.len() - 1;
        self.dirty = true;
        self.status = "Variable added — press S to save".into();
    }

    pub fn begin_delete_confirmation(&mut self) {
        match self.entries.get(self.selected) {
            Some(entry) if entry.source == EnvSource::System && !self.system_write_allowed => {
                self.status = SYSTEM_ACCESS_WARNING.into();
            }
            Some(_) => self.delete_confirmation = Some(self.selected),
            None => {}
        }
    }

    pub fn cancel_delete(&mut self) {
        self.delete_confirmation = None;
    }

    pub fn confirm_delete(&mut self) {
        if let Some(index) = self.delete_confirmation.take() {
            if index < self.entries.len() {
                let key = self.entries.remove(index).key;
                self.selected = self.selected.min(self.entries.len().saturating_sub(1));
                self.dirty = true;
                self.status = format!("Variable {key} deleted — press S to save");
            }
        }
    }

    pub fn begin_edit(&mut self) {
        if self
            .entries
            .get(self.selected)
            .is_some_and(|entry| entry.source == EnvSource::System && !self.system_write_allowed)
        {
            self.status = SYSTEM_ACCESS_WARNING.into();
            return;
        }
        if let Some(entry) = self.entries.get(self.selected) {
            self.edit_as_list = entry.key.eq_ignore_ascii_case("PATH") || entry.value.contains(';');
            if self.edit_as_list {
                // Empty PATH components (for example from `;;` or a trailing `;`)
                // otherwise render as indistinguishable blank rows in the editor.
                self.list_items = Some(
                    entry
                        .value
                        .split(';')
                        .filter(|item| !item.is_empty())
                        .map(str::to_owned)
                        .collect(),
                );
                self.list_selected = 0;
                self.edit_buffer = None;
                self.status = format!(
                    "{}: ↑/↓ select, Enter edit, a add, d delete, Ctrl+Enter apply",
                    entry.key
                );
            } else {
                self.edit_cursor = entry.value.chars().count();
                self.edit_buffer = Some(entry.value.clone());
                self.status = format!("Edit {} — Enter: apply, Esc: cancel", entry.key);
            }
        }
    }

    pub fn begin_list_item_edit(&mut self) {
        if let Some(item) = self
            .list_items
            .as_ref()
            .and_then(|items| items.get(self.list_selected))
        {
            self.edit_cursor = item.chars().count();
            self.edit_buffer = Some(item.clone());
            self.editing_list_item = true;
        }
    }

    pub fn add_list_item(&mut self) {
        if let Some(items) = self.list_items.as_mut() {
            items.push(String::new());
            self.list_selected = items.len() - 1;
            self.begin_list_item_edit();
        }
    }

    pub fn delete_list_item(&mut self) {
        if let Some(items) = self.list_items.as_mut() {
            if !items.is_empty() {
                items.remove(self.list_selected);
                self.list_selected = self.list_selected.min(items.len().saturating_sub(1));
            }
        }
    }

    pub fn move_list_selection(&mut self, down: bool) {
        if let Some(items) = self.list_items.as_ref() {
            if down {
                self.list_selected = (self.list_selected + 1).min(items.len().saturating_sub(1));
            } else {
                self.list_selected = self.list_selected.saturating_sub(1);
            }
        }
    }

    pub fn page_list_selection(&mut self, down: bool, page_size: usize) {
        if let Some(items) = self.list_items.as_ref() {
            let last = items.len().saturating_sub(1);
            let amount = page_size.max(1);
            self.list_selected = if down {
                self.list_selected.saturating_add(amount).min(last)
            } else {
                self.list_selected.saturating_sub(amount)
            };
        }
    }

    pub fn reorder_list_item(&mut self, down: bool) {
        if let Some(items) = self.list_items.as_mut() {
            let target = if down {
                self.list_selected
                    .checked_add(1)
                    .filter(|index| *index < items.len())
            } else {
                self.list_selected.checked_sub(1)
            };
            if let Some(target) = target {
                items.swap(self.list_selected, target);
                self.list_selected = target;
            }
        }
    }

    pub fn edit_char(&mut self, character: char) {
        if let Some(buffer) = self.edit_buffer.as_mut() {
            let byte_index = buffer
                .char_indices()
                .nth(self.edit_cursor)
                .map(|(index, _)| index)
                .unwrap_or(buffer.len());
            buffer.insert(byte_index, character);
            self.edit_cursor += 1;
        }
    }

    pub fn edit_backspace(&mut self) {
        if let Some(buffer) = self.edit_buffer.as_mut() {
            if self.edit_cursor > 0 {
                let start = buffer
                    .char_indices()
                    .nth(self.edit_cursor - 1)
                    .map(|(index, _)| index)
                    .unwrap_or(buffer.len());
                let end = buffer
                    .char_indices()
                    .nth(self.edit_cursor)
                    .map(|(index, _)| index)
                    .unwrap_or(buffer.len());
                buffer.replace_range(start..end, "");
                self.edit_cursor -= 1;
            }
        }
    }

    pub fn edit_delete(&mut self) {
        if let Some(buffer) = self.edit_buffer.as_mut() {
            if let Some(start) = buffer.char_indices().nth(self.edit_cursor).map(|(i, _)| i) {
                let end = buffer
                    .char_indices()
                    .nth(self.edit_cursor + 1)
                    .map(|(i, _)| i)
                    .unwrap_or(buffer.len());
                buffer.replace_range(start..end, "");
            }
        }
    }

    pub fn edit_move_left(&mut self) {
        self.edit_cursor = self.edit_cursor.saturating_sub(1);
    }

    pub fn edit_move_right(&mut self) {
        if let Some(buffer) = self.edit_buffer.as_ref() {
            self.edit_cursor = (self.edit_cursor + 1).min(buffer.chars().count());
        }
    }

    pub fn edit_move_up(&mut self, display_width: usize) {
        self.edit_move_visual_line(display_width, false);
    }

    pub fn edit_move_down(&mut self, display_width: usize) {
        self.edit_move_visual_line(display_width, true);
    }

    pub fn edit_cursor_position(&self, display_width: usize) -> (usize, usize) {
        let Some(buffer) = self.edit_buffer.as_ref() else {
            return (0, 0);
        };
        cursor_positions(buffer, display_width)
            .get(self.edit_cursor)
            .copied()
            .unwrap_or((0, 0))
    }

    fn edit_move_visual_line(&mut self, display_width: usize, down: bool) {
        let Some(buffer) = self.edit_buffer.as_ref() else {
            return;
        };
        let positions = cursor_positions(buffer, display_width);
        let Some(&(row, column)) = positions.get(self.edit_cursor) else {
            return;
        };
        let target_row = if down {
            row + 1
        } else if row > 0 {
            row - 1
        } else {
            return;
        };
        if let Some((index, _)) = positions
            .iter()
            .enumerate()
            .filter(|(_, (candidate_row, _))| *candidate_row == target_row)
            .min_by_key(|(_, (_, candidate_column))| candidate_column.abs_diff(column))
        {
            self.edit_cursor = index;
        }
    }

    pub fn edit_home(&mut self) {
        if let Some(buffer) = self.edit_buffer.as_ref() {
            let chars: Vec<char> = buffer.chars().collect();
            self.edit_cursor = chars[..self.edit_cursor.min(chars.len())]
                .iter()
                .rposition(|character| *character == '\n')
                .map_or(0, |index| index + 1);
        }
    }

    pub fn edit_end(&mut self) {
        if let Some(buffer) = self.edit_buffer.as_ref() {
            let chars: Vec<char> = buffer.chars().collect();
            let cursor = self.edit_cursor.min(chars.len());
            let line_start = chars[..cursor]
                .iter()
                .rposition(|character| *character == '\n')
                .map_or(0, |index| index + 1);
            self.edit_cursor = chars
                .iter()
                .enumerate()
                .skip(line_start)
                .find(|(_, character)| **character == '\n')
                .map_or(chars.len(), |(index, _)| index);
        }
    }

    pub fn commit_edit(&mut self) {
        if self.editing_list_item {
            if let (Some(buffer), Some(items)) = (self.edit_buffer.take(), self.list_items.as_mut())
            {
                if let Some(item) = items.get_mut(self.list_selected) {
                    *item = buffer;
                }
            }
            self.editing_list_item = false;
            return;
        }

        if self.edit_as_list {
            let value = self
                .list_items
                .take()
                .unwrap_or_default()
                .into_iter()
                .filter(|item| !item.is_empty())
                .collect::<Vec<_>>()
                .join(";");
            if let Some(entry) = self.entries.get_mut(self.selected) {
                if entry.value != value {
                    entry.value = value;
                    self.dirty = true;
                }
            }
            self.edit_as_list = false;
            self.status = if self.dirty {
                "Unsaved changes — press S to save".into()
            } else {
                "No changes".into()
            };
            return;
        }

        if let (Some(buffer), Some(entry)) =
            (self.edit_buffer.take(), self.entries.get_mut(self.selected))
        {
            if entry.value != buffer {
                entry.value = buffer;
                self.dirty = true;
            }
            self.status = if self.dirty {
                "Unsaved changes — press S to save".into()
            } else {
                "No changes".into()
            };
        }
    }

    pub fn cancel_edit(&mut self) {
        if self.editing_list_item {
            self.edit_buffer = None;
            self.editing_list_item = false;
        } else if self.list_items.take().is_some() {
            self.edit_as_list = false;
            self.status = "List edit canceled".into();
        } else if self.edit_buffer.take().is_some() {
            self.edit_as_list = false;
            self.status = "Edit canceled".into();
        }
    }

    pub fn mark_saved(&mut self) {
        self.original_entries.clone_from(&self.entries);
        self.dirty = false;
        self.status = "Changes saved".into();
    }
}

fn cursor_positions(value: &str, display_width: usize) -> Vec<(usize, usize)> {
    let display_width = display_width.max(1);
    let mut positions = vec![(0, 0)];
    let (mut row, mut column) = (0, 0);

    for (index, character) in value.chars().enumerate() {
        if character == '\n' {
            row += 1;
            column = 0;
            positions.push((row, column));
            continue;
        }

        let character_width = UnicodeWidthChar::width(character).unwrap_or(0);
        if character_width > 0 && column > 0 && column + character_width > display_width {
            row += 1;
            column = 0;
            positions[index] = (row, column);
        }
        column += character_width;
        if column >= display_width {
            row += 1;
            column = 0;
        }
        positions.push((row, column));
    }
    positions
}

#[cfg(test)]
mod tests {
    use super::{App, EnvEntry, EnvSource};

    fn app_with_entries(count: usize) -> App {
        let entries = (0..count)
            .map(|index| EnvEntry {
                key: format!("KEY_{index}"),
                value: format!("value_{index}"),
                source: EnvSource::Dotenv,
                expandable: false,
                leading_lines: Vec::new(),
                trailing_lines: Vec::new(),
            })
            .collect();
        App::new(entries)
    }

    #[test]
    fn selection_moves_forward_and_stops_at_last_entry() {
        let mut app = app_with_entries(2);

        app.select_next();
        assert_eq!(app.selected, 1);

        app.select_next();
        assert_eq!(app.selected, 1);
    }

    #[test]
    fn selection_moves_backward_and_stops_at_first_entry() {
        let mut app = app_with_entries(2);

        app.select_previous();
        assert_eq!(app.selected, 0);

        app.select_previous();
        assert_eq!(app.selected, 0);
    }

    #[test]
    fn selection_stays_at_zero_when_there_are_no_entries() {
        let mut app = app_with_entries(0);

        app.select_next();
        app.select_previous();

        assert_eq!(app.selected, 0);
    }
}
