use ratatui::layout::{Constraint, Direction, Layout, Rect};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LayoutMode {
    Full,
    Compact,
    TooSmall,
}

#[derive(Debug, Clone, Copy)]
pub struct AppLayout {
    pub mode: LayoutMode,
    pub header: Rect,
    pub content: Rect,
    pub rail: Rect,
    pub footer: Rect,
}

impl AppLayout {
    #[must_use]
    pub fn calculate(area: Rect) -> Self {
        if area.width < 60 || area.height < 20 {
            return Self {
                mode: LayoutMode::TooSmall,
                header: area,
                content: Rect::default(),
                rail: Rect::default(),
                footer: Rect::default(),
            };
        }
        let vertical = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(2),
                Constraint::Min(8),
                Constraint::Length(1),
            ])
            .split(area);
        let rail_width = if area.width >= 120 { 15 } else { 12 };
        let horizontal = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Min(40), Constraint::Length(rail_width)])
            .split(vertical[1]);
        Self {
            mode: if area.width >= 120 && area.height >= 36 {
                LayoutMode::Full
            } else {
                LayoutMode::Compact
            },
            header: vertical[0],
            content: horizontal[0],
            rail: horizontal[1],
            footer: vertical[2],
        }
    }
}

#[must_use]
pub fn split_exact(area: Rect, columns: u16, rows: u16) -> Vec<Rect> {
    if columns == 0 || rows == 0 {
        return Vec::new();
    }
    let column_width = area.width / columns;
    let extra_columns = area.width % columns;
    let row_height = area.height / rows;
    let extra_rows = area.height % rows;
    let mut result = Vec::with_capacity(usize::from(columns * rows));
    let mut y = area.y;
    for row in 0..rows {
        let height = row_height + u16::from(row < extra_rows);
        let mut x = area.x;
        for column in 0..columns {
            let width = column_width + u16::from(column < extra_columns);
            result.push(Rect::new(x, y, width, height));
            x = x.saturating_add(width);
        }
        y = y.saturating_add(height);
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn split_exact_covers_odd_sized_area() {
        let area = Rect::new(2, 3, 101, 41);
        let cells = split_exact(area, 3, 3);
        assert_eq!(cells.len(), 9);
        assert_eq!(cells[0].x, 2);
        assert_eq!(cells[8].right(), area.right());
        assert_eq!(cells[8].bottom(), area.bottom());
    }

    #[test]
    fn small_terminal_is_rejected() {
        assert_eq!(
            AppLayout::calculate(Rect::new(0, 0, 59, 20)).mode,
            LayoutMode::TooSmall
        );
        assert_eq!(
            AppLayout::calculate(Rect::new(0, 0, 60, 20)).mode,
            LayoutMode::Compact
        );
    }
}
