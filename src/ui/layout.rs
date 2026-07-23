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
pub fn uniform_grid(area: Rect, columns: u16, rows: u16) -> Vec<Rect> {
    if columns == 0 || rows == 0 {
        return Vec::new();
    }
    let cell_width = area.width / columns;
    let cell_height = area.height / rows;
    if cell_width == 0 || cell_height == 0 {
        return Vec::new();
    }

    let used_width = cell_width * columns;
    let used_height = cell_height * rows;
    let origin_x = area.x + (area.width - used_width) / 2;
    let origin_y = area.y + (area.height - used_height) / 2;
    let mut result = Vec::with_capacity(usize::from(columns * rows));
    for row in 0..rows {
        for column in 0..columns {
            result.push(Rect::new(
                origin_x + column * cell_width,
                origin_y + row * cell_height,
                cell_width,
                cell_height,
            ));
        }
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn uniform_grid_centers_equal_cells_and_leaves_remainder_as_padding() {
        let area = Rect::new(2, 3, 101, 41);
        let cells = uniform_grid(area, 3, 3);

        assert_eq!(cells.len(), 9);
        assert!(cells.iter().all(|cell| cell.width == 33));
        assert!(cells.iter().all(|cell| cell.height == 13));
        assert_eq!(cells[0], Rect::new(3, 4, 33, 13));
        assert_eq!(cells[8].right(), 102);
        assert_eq!(cells[8].bottom(), 43);
        assert_eq!(cells[0].x - area.x, area.right() - cells[8].right());
        assert_eq!(cells[0].y - area.y, area.bottom() - cells[8].bottom());
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
